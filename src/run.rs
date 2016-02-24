//! Launching and running the script

use ast::{Script, Trigger, Statement, Conjunction, Condition, UncheckedCtx, UncheckedEnv};
use compile::{Compiler, CompiledCtx, ExecutableDevEnv};
use compile;

use fxbox_taxonomy::values::*;
use fxbox_taxonomy::api;
use fxbox_taxonomy::api::{API, WatchEvent};

use std::sync::mpsc::{channel, Receiver, Sender};
use std::marker::PhantomData;
use std::result::Result;
use std::result::Result::*;
use std::thread;
use std::sync::Arc;
use std::collections::HashMap;

use chrono::UTC;

fn test<F>(cb: F) where F: FnMut(WatchEvent) + Send + Sync {
    unimplemented!()
}


/// Running and controlling a single script.
pub struct Execution<Env> where Env: ExecutableDevEnv + 'static {
    command_sender: Option<Sender<ExecutionOp>>,
    phantom: PhantomData<Env>,
}

impl<Env> Execution<Env> where Env: ExecutableDevEnv + 'static {
    pub fn new() -> Self {
        Execution {
            command_sender: None,
            phantom: PhantomData,
        }
    }

    /// Start executing the script.
    ///
    /// # Errors
    ///
    /// Produces RunningError:AlreadyRunning if the script is already running.
    pub fn start<F>(&mut self, script: Script<UncheckedCtx, UncheckedEnv>, on_result: F) where F: FnOnce(Result<(), Error>) + Send + 'static {
        if self.command_sender.is_some() {
            on_result(Err(Error::RunningError(RunningError::AlreadyRunning)));
            return;
        }
        let (tx, rx) = channel();
        let tx2 = tx.clone();
        self.command_sender = Some(tx);
        thread::spawn(move || {
            match ExecutionTask::<Env>::new(script, tx2, rx) {
                Err(er) => {
                    on_result(Err(er));
                },
                Ok(mut task) => {
                    on_result(Ok(()));
                    task.run();
                }
            }
        });
    }


    /// Stop executing the script, asynchronously.
    ///
    /// # Errors
    ///
    /// Produces RunningError:NotRunning if the script is not running yet.
    pub fn stop<F>(&mut self, on_result: F) where F: Fn(Result<(), Error>) + Send + 'static {
        match self.command_sender {
            None => {
                /* Nothing to stop */
                on_result(Err(Error::RunningError(RunningError::NotRunning)));
            },
            Some(ref tx) => {
                // Shutdown the application, asynchronously.
                let _ignored = tx.send(ExecutionOp::Stop(Box::new(on_result)));
            }
        };
        self.command_sender = None;
    }
}

impl<Env> Drop for Execution<Env> where Env: ExecutableDevEnv + 'static {
    fn drop(&mut self) {
        let _ignored = self.stop(|_ignored| { });
    }
}

/// A script ready to be executed. Each script is meant to be
/// executed in an individual thread.
pub struct ExecutionTask<Env> where Env: ExecutableDevEnv {
    /// The script, annotated with its state.
    state: Script<CompiledCtx<Env>, Env>,

    /// Communicating with the thread running script.
    tx: Sender<ExecutionOp>,
    rx: Receiver<ExecutionOp>,
}





enum ExecutionOp {
    /// An input has been updated, time to check if we have triggers
    /// ready to be executed.
    Update {index: usize, value: Value},

    /// Time to stop executing the script.
    Stop(Box<Fn(Result<(), Error>) + Send>)
}


impl<Env> ExecutionTask<Env> where Env: ExecutableDevEnv {
    /// Create a new execution task.
    ///
    /// The caller is responsible for spawning a new thread and
    /// calling `run()`.
    fn new(script: Script<UncheckedCtx, UncheckedEnv>, tx: Sender<ExecutionOp>, rx: Receiver<ExecutionOp>) -> Result<Self, Error> {
        let compiler = try!(Compiler::new().map_err(|err| Error::CompileError(err)));
        let state = try!(compiler.compile(script).map_err(|err| Error::CompileError(err)));
        
        Ok(ExecutionTask {
            state: state,
            rx: rx,
            tx: tx
        })
    }

    /// Execute the monitoring task.
    /// This currently expects to be executed in its own thread.
    fn run(&mut self) {
        let mut witnesses = Vec::new();

        // Start listening to all inputs that appear in conditions.
        for rule in &self.state.rules  {
            for condition in &rule.condition.all {

                // The latest values received from the inputs.
                let mut values = HashMap::new();

                let options = api::WatchOptions::new()
                    .with_inputs(condition.input.clone());
                // We will often end up watching several times the
                // same service. For the moment, we do not attempt to
                // optimize either I/O (which we expect will be
                // optimized by `watcher`) or condition checking
                // (which we should eventually optimize, if we find
                // out that we end up with large rulesets).
                witnesses.push(
                    Env::API::register_service_watch(
                        options,
                        move |event| {
                            match event {
                                WatchEvent::InputRemoved(id) => {
                                    values.remove(&id);
                                },
                                WatchEvent::InputAdded(id) => {
                                    // An input was added. Note that there is
                                    // a possibility that the input was not
                                    // empty, in case we received messages in
                                    // the wrong order.
                                    values.entry(id).or_insert(None);
                                }
                                WatchEvent::Value{from: id, value} => {
                                    // An input was updated. Note that there is
                                    // a possibility that the input was
                                    // empty, in case we received messages in
                                    // the wrong order.
                                    values.insert(id, Some(value));
                                    // FIXME: Now check whether the condition is met.
                                }
                            }
                        }));
                }
                        
        }
/*
        // Make sure that the vector never mutates past this
        // point. This ensures that our `index` remains valid for the
        // rest of the execution.
        let cells = cells;

        // FIXME: We are going to end up with stale data in some inputs.
        // We need to find out how to get rid of it.

        // Now, start handling events.
        for msg in &self.rx {
            use self::ExecutionOp::*;
            match msg {
                Stop(f) => {
                    // Leave the loop.
                    // The watcher and the witnesses will be cleaned up on exit.
                    // Any further message will be ignored.
                    f(Ok(()));
                    return;
                }

                Update {value, index} => {
                    let cell = &cells[index];
                    *cell.state.write().unwrap() = Some(DatedData {
                        updated: UTC::now(),
                        data: value
                    });
                    // Note that we can unwrap() safely,
                    // as it fails only if the thread is
                    // already in panic.

                    // Find out if we should execute triggers.
                    for mut rule in &mut self.state.rules {
                        let is_met = rule.is_met();
                        if !(is_met.new && !is_met.old) {
                            // We should execute the trigger only if
                            // it was false and is now true. Here,
                            // either it was already true or it isn't
                            // false yet.
                            continue;
                        }

                        // Conditions were not met, now they are, so
                        // it is time to start executing.

                        // FIXME: We do not want triggers to be
                        // triggered too often. Handle cooldown.
                        
                        for statement in &rule.execute {
                            let _ignored = statement.eval(); // FIXME: Log errors
                        }
                    }
                }

            }
        }
*/

    }
}

/*
///
/// # Evaluating conditions
///

struct IsMet {
    old: bool,
    new: bool,
}

impl<Env> Trigger<CompiledCtx<Env>, Env> where Env: DevEnv {
    fn is_met(&mut self) -> IsMet {
        self.condition.is_met()
    }
}


impl<Env> Conjunction<CompiledCtx<Env>, Env> where Env: DevEnv {
    /// For a conjunction to be true, all its components must be true.
    fn is_met(&mut self) -> IsMet {
        let old = self.state.is_met;
        let mut new = true;

        for mut single in &mut self.all {
            if !single.is_met().new {
                new = false;
                // Don't break. We want to make sure that we update
                // `is_met` of all individual conditions.
            }
        }
        self.state.is_met = new;
        IsMet {
            old: old,
            new: new,
        }
    }
}


impl<Env> Condition<CompiledCtx<Env>, Env> where Env: DevEnv {
    /// Determine if one of the devices serving as input for this
    /// condition meets the condition.
    fn is_met(&mut self) -> IsMet {
        let old = self.state.is_met;
        let mut new = false;
        for single in &*self.input {
            // This will fail only if the thread has already panicked.
            let state = single.state.read().unwrap();
            let is_met = match *state {
                None => { false /* We haven't received a measurement yet.*/ },
                Some(ref data) => {
                    self.range.contains(&data.data)
                }
            };
            if is_met {
                new = true;
                break;
            }
        }

        self.state.is_met = new;
        IsMet {
            old: old,
            new: new,
        }
    }
}

impl<Env> Statement<CompiledCtx<Env>, Env> where Env: ExecutableDevEnv {
    fn eval(&self) -> Result<(), Error> {
        let args = self.arguments.iter().map(|(k, v)| {
            (k.clone(), v.eval())
        }).collect();
        for output in &self.destination {
            Env::send(&output.device, &self.action, &args); // FIXME: Handle errors
        }
        return Ok(());
    }
}

/*
impl<Env> Expression<CompiledCtx<Env>, Env> where Env: ExecutableDevEnv {
    fn eval(&self) -> Value {
        match *self {
            Expression::Value(ref v) => v.clone(),
            Expression::Input(_) => panic!("Cannot read an input in an expression yet"),
            Expression::Vec(_) => {
                panic!("Cannot handle vectors of expressions yet");
            }
        }
    }
}
*/
*/

#[derive(Debug)]
pub enum RunningError {
    AlreadyRunning,
    NotRunning,
}

#[derive(Debug)]
pub enum Error {
    CompileError(compile::Error),
    RunningError(RunningError),
}


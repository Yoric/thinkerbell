//! Launching and running the script

use ast::{ Script, Statement, UncheckedCtx} ;
use compile::{ Compiler, CompiledCtx, ExecutableDevEnv} ;
use compile;

use foxbox_taxonomy::api;
use foxbox_taxonomy::api::{ API, Error as APIError, WatchEvent };
use foxbox_taxonomy::services::{ Getter, Setter };
use foxbox_taxonomy::util::{ Exactly, Id };
use foxbox_taxonomy::values::Range;

use transformable_channels::mpsc::*;

use std::marker::PhantomData;
use std::thread;
use std::collections::HashMap;

/// Running and controlling a single script.
pub struct Execution<Env> where Env: ExecutableDevEnv + 'static {
    command_sender: Option<Box<ExtSender<ExecutionOp>>>,
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
    /// # Memory warning
    ///
    /// If you do not consume the values from `on_event`, they will remain stored forever.
    /// You have been warned.
    ///
    /// # Errors
    ///
    /// The first event sent to `on_event` is a `ExecutionEvent::Starting`, which informs the
    /// caller of whether the execution could start. Possible reasons that would prevent execution
    /// are:
    /// - `RunningError:AlreadyRunning` if the script is already running;
    /// - a compilation error if the script was incorrect.
    pub fn start<S>(&mut self, api: Env::API, script: Script<UncheckedCtx>, on_event: S) ->
        Result<(), Error>
        where S: ExtSender<ExecutionEvent>
    {
        if self.command_sender.is_some() {
            let err = Err(Error::StartStopError(StartStopError::AlreadyRunning));
            let _ = on_event.send(ExecutionEvent::Starting {
                result: err.clone()
            });
            err
        } else {
            // One-time channel, used to wait until compilation is complete.
            let (tx_init, rx_init) = channel();

            let (tx, rx) = channel();
            self.command_sender = Some(Box::new(tx.clone()));
            thread::spawn(move || {
                match ExecutionTask::<Env>::new(script, tx, rx) {
                    Err(er) => {
                        let _ = on_event.send(ExecutionEvent::Starting {
                            result: Err(er.clone())
                        });
                        let _ = tx_init.send(Err(er));
                    },
                    Ok(mut task) => {
                        let _ = on_event.send(ExecutionEvent::Starting {
                            result: Ok(())
                        });
                        let _ = tx_init.send(Ok(()));
                        task.run(api, on_event);
                    }
                }
            });
            match rx_init.recv() {
                Ok(result) => result,
                Err(_) => Err(Error::StartStopError(StartStopError::ThreadError))
            }
        }
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
                on_result(Err(Error::StartStopError(StartStopError::NotRunning)));
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
    script: Script<CompiledCtx<Env>>,

    /// Communicating with the thread running script.
    tx: Box<ExtSender<ExecutionOp>>,
    rx: Receiver<ExecutionOp>,
}

#[derive(Debug)]
pub enum ExecutionEvent {
    Starting {
        result: Result<(), Error>,
    },
    Stopped {
        result: Result<(), Error>
    },
    Updated {
        event: WatchEvent,
        rule_index: usize,
        condition_index: usize
    },
    Sent {
        rule_index: usize,
        statement_index: usize,
        result: Vec<(Id<Setter>, Result<(), Error>)>
    },
    ChannelError {
        id: Id<Getter>,
        error: APIError,
    }
}

enum ExecutionOp {
    Update { event: WatchEvent, rule_index: usize, condition_index: usize },
    /// Time to stop executing the script.
    Stop(Box<Fn(Result<(), Error>) + Send>)
}


impl<Env> ExecutionTask<Env> where Env: ExecutableDevEnv {
    /// Create a new execution task.
    ///
    /// The caller is responsible for spawning a new thread and
    /// calling `run()`.
    fn new<S>(script: Script<UncheckedCtx>, tx: S, rx: Receiver<ExecutionOp>) -> Result<Self, Error>
        where S: ExtSender<ExecutionOp>
    {
        let compiler = try!(Compiler::new().map_err(|err| Error::CompileError(err)));
        let script = try!(compiler.compile(script).map_err(|err| Error::CompileError(err)));

        Ok(ExecutionTask {
            script: script,
            rx: rx,
            tx: Box::new(tx)
        })
    }

    /// Execute the monitoring task.
    /// This currently expects to be executed in its own thread.
    fn run<S>(&mut self, api: Env::API, on_event: S) where S: ExtSender<ExecutionEvent> {
        let mut witnesses = Vec::new();

        struct ConditionState {
            match_is_met: bool,
            per_getter: HashMap<Id<Getter>, bool>,
            range: Range,
        };
        struct RuleState {
            rule_is_met: bool,
            per_condition: Vec<ConditionState>,
        };

        // Generate the state of rules, conditions, getters and start
        // listening to changes in the getters.

        // FIXME: We could optimize requests by grouping per `TargetMap<GetterSelector, Exactly<Range>>`
        let mut per_rule : Vec<_> = self.script.rules.iter().zip(0 as usize..).map(|(rule, rule_index)| {
            let per_condition = rule.conditions.iter().zip(0 as usize..).map(|(condition, condition_index)| {
                // We will often end up watching several times the
                // same channel. For the moment, we do not attempt to
                // optimize either I/O (which we expect will be
                // optimized by `watcher`) or condition checking
                // (which we should eventually optimize, if we find
                // out that we end up with large rulesets).

                let rule_index = rule_index.clone();
                let condition_index = condition_index.clone();
                witnesses.push(
                    api.watch_values(
                        vec![(condition.source.clone(), Exactly::Exactly(condition.range.clone())) ],
                        Box::new(self.tx.map(move |event| {
                            ExecutionOp::Update {
                                event: event,
                                rule_index: rule_index,
                                condition_index: condition_index
                            }
                        }))));
                let range = condition.range.clone();
                ConditionState {
                    match_is_met: false,
                    per_getter: HashMap::new(),
                    range: range,
                }
            }).collect();

            RuleState {
                rule_is_met: false,
                per_condition: per_condition
            }
        }).collect();

        for msg in self.rx.iter() {
            match msg {
                ExecutionOp::Stop(cb) => {
                    // Leave the loop. Watching will stop once
                    // `witnesses` is dropped.
                    cb(Ok(()));
                    return;
                },
                ExecutionOp::Update {
                    event,
                    rule_index,
                    condition_index,
                } => match event {
                    WatchEvent::InitializationError {
                        channel,
                        error
                    } => {
                        let _ = on_event.send(ExecutionEvent::ChannelError {
                            id: channel,
                            error: error,
                        });
                    },
                    WatchEvent::GetterRemoved(id) => {
                        per_rule[rule_index]
                            .per_condition[condition_index]
                            .per_getter
                            .remove(&id);
                    },
                    WatchEvent::GetterAdded(id) => {
                        // An getter was added. Note that there is
                        // a possibility that the getter was not
                        // empty, in case we received messages in
                        // the wrong order.
                        per_rule[rule_index]
                            .per_condition[condition_index]
                            .per_getter
                            .insert(id, false);
                    }
                    WatchEvent::EnterRange { from: id, value }
                    | WatchEvent::ExitRange { from: id, value }
                        // FIXME: EnterRange/ExitRange would let us simplify condition checking 
                    => {
                        use std::mem::replace;

                        // An getter was updated. Note that there is
                        // a possibility that the getter was
                        // empty, in case we received messages in
                        // the wrong order.

                        let getter_is_met : bool =
                            per_rule[rule_index]
                            .per_condition[condition_index]
                            .range
                            .contains(&value);

                        per_rule[rule_index]
                            .per_condition[condition_index]
                            .per_getter
                            .insert(id, getter_is_met); // FIXME: Could be used to optimize

                        // 1. Is the match met?
                        //
                        // The match is met iff any of the getters
                        // meets the condition.
                        let some_getter_is_met = getter_is_met ||
                            per_rule[rule_index]
                            .per_condition[condition_index]
                            .per_getter
                            .values().find(|is_met| **is_met).is_some();

                        per_rule[rule_index]
                            .per_condition[condition_index]
                            .match_is_met = some_getter_is_met;

                        // 2. Is the condition met?
                        //
                        // The condition is met iff all of the
                        // matches are met.
                        let condition_is_met =
                            per_rule[rule_index]
                            .per_condition
                            .iter()
                            .find(|condition_state| condition_state.match_is_met)
                            .is_some();

                        // 3. Are we in a case in which the
                        // condition was not met and is now met?
                        let condition_was_met =
                            replace(&mut per_rule[rule_index].rule_is_met, condition_is_met);

                        if !condition_was_met && condition_is_met {
                            // Ahah, we have just triggered the statements!
                            for (statement, statement_index) in self.script.rules[rule_index].execute.iter().zip(0..) {
                                let result = statement.eval(&api);
                                let _ = on_event.send(ExecutionEvent::Sent {
                                    rule_index: rule_index,
                                    statement_index: statement_index,
                                    result: result,
                                });
                            }
                        }
                    }
                }
            };
        }
    }
}


impl<Env> Statement<CompiledCtx<Env>> where Env: ExecutableDevEnv {
    fn eval(&self, api: &Env::API) ->  Vec<(Id<Setter>, Result<(), Error>)> {
        api.send_values(vec![(self.destination.clone(), self.value.clone())])
            .into_iter()
            .map(|(id, result)|
                 (id, result.map_err(|err| Error::APIError(err))))
            .collect()
    }
}



#[derive(Clone, Debug, Serialize)]
pub enum StartStopError {
    AlreadyRunning,
    NotRunning,
    ThreadError,
}

#[derive(Clone, Debug, Serialize)]
pub enum Error {
    CompileError(compile::Error),
    StartStopError(StartStopError),
    APIError(api::Error),
}


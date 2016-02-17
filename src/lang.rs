#![allow(unused_variables)]
#![allow(dead_code)]

/// Basic structure of a Monitor (aka Server App)
///
/// Monitors are designed so that the FoxBox can offer a simple
/// IFTTT-style Web UX to let users write their own scripts. More
/// complex monitors can installed from the web from a master device
/// (i.e. the user's cellphone or smart tv).

use dependencies::{DevEnv, ExecutableDevEnv, Watcher};
use values::{Value, Range};
use util::map;

use std::collections::HashMap;
use std::sync::{Arc, RwLock}; // FIXME: Investigate if we really need so many instances of Arc. I suspect that most can be replaced by &'a.
use std::sync::mpsc::{channel, Receiver, Sender};
use std::marker::PhantomData;
use std::result::Result;
use std::result::Result::*;
use std::thread;

extern crate chrono;
use self::chrono::{DateTime, UTC};


///
/// # Definition of the AST
///


/// A Monitor Application, i.e. an application (or a component of an
/// application) executed on the server.
///
/// Monitor applications are typically used for triggering an action
/// in reaction to an event: changing temperature when night falls,
/// ringing an alarm when a door is opened, etc.
///
/// Monitor applications are installed from a paired device. They may
/// either be part of a broader application (which can install them
/// through a web/REST API) or live on their own.
pub struct Script<Ctx, Env> where Env: DevEnv, Ctx: Context {
    /// Authorization, author, description, update url, version, ...
    pub metadata: (), // FIXME: Implement

    /// Monitor applications have sets of requirements (e.g. "I need a
    /// camera"), which are allocated to actual resources through the
    /// UX. Re-allocating resources may be requested by the user, the
    /// foxbox, or an application, e.g. when replacing a device or
    /// upgrading the app.
    pub requirements: Vec<Requirement<Ctx, Env>>,

    /// Resources actually allocated for each requirement.
    /// This must have the same size as `requirements`.
    pub allocations: Vec<Resource<Ctx, Env>>,

    /// A set of rules, stating what must be done in which circumstance.
    pub rules: Vec<Trigger<Ctx, Env>>,
}

pub struct Resource<Ctx, Env> where Env: DevEnv, Ctx: Context {
    pub devices: Vec<Env::Device>,
    pub phantom: PhantomData<Ctx>,
}


/// A resource needed by this application. Typically, a definition of
/// device with some input our output capabilities.
pub struct Requirement<Ctx, Env> where Env: DevEnv, Ctx: Context {
    /// The kind of resource, e.g. "a flashbulb".
    pub kind: Env::DeviceKind,

    /// Input capabilities we need from the device, e.g. "the time of
    /// day", "the current temperature".
    pub inputs: Vec<Env::InputCapability>,

    /// Output capabilities we need from the device, e.g. "play a
    /// sound", "set luminosity".
    pub outputs: Vec<Env::OutputCapability>,
    
    pub phantom: PhantomData<Ctx>,
    // FIXME: We may need cooldown properties.
}

/// A single trigger, i.e. "when some condition becomes true, do
/// something".
pub struct Trigger<Ctx, Env> where Env: DevEnv, Ctx: Context {
    /// The condition in which to execute the trigger.
    pub condition: Conjunction<Ctx, Env>,

    /// Stuff to do once `condition` is met.
    pub execute: Vec<Statement<Ctx, Env>>,

    /*
    /// Minimal duration between two executions of the trigger.  If a
    /// duration was not picked by the developer, a reasonable default
    /// duration should be picked (e.g. 10 minutes).
    FIXME: Implement
    pub cooldown: Duration,
     */
}

/// A conjunction (e.g. a "and") of conditions.
pub struct Conjunction<Ctx, Env> where Env: DevEnv, Ctx: Context {
    /// The conjunction is true iff all of the following expressions evaluate to true.
    pub all: Vec<Condition<Ctx, Env>>,
    pub state: Ctx::ConditionState,
}

/// An individual condition.
///
/// Conditions always take the form: "data received from sensor is in
/// given range".
///
/// A condition is true if *any* of the sensors allocated to this
/// requirement has yielded a value that is in the given range.
pub struct Condition<Ctx, Env> where Env: DevEnv, Ctx: Context {
    pub input: Ctx::InputSet,
    pub capability: Env::InputCapability,
    pub range: Range,
    pub state: Ctx::ConditionState,
}


/// Stuff to actually do. In practice, this means placing calls to devices.
pub struct Statement<Ctx, Env> where Env: DevEnv, Ctx: Context {
    /// The resource to which this command applies.  e.g. "all
    /// heaters", "a single communication channel", etc.
    pub destination: Ctx::OutputSet,

    /// The action to execute on the resource.
    pub action: Env::OutputCapability,

    /// Data to send to the resource.
    pub arguments: HashMap<String, Expression<Ctx, Env>>
}

pub struct InputSet<Ctx, Env> where Env: DevEnv, Ctx: Context {
    /// The set of inputs from which to grab the value, i.e.
    /// all the inputs matching some condition.
    pub condition: Condition<Ctx, Env>,

    /// The value to grab.
    pub capability: Env::InputCapability,
}

/// A value that may be sent to an output.
pub enum Expression<Ctx, Env> where Env: DevEnv, Ctx: Context {
    /// A dynamic value, which must be read from one or more inputs.
    // FIXME: Not ready yet
    Input(InputSet<Ctx, Env>),

    /// A constant value.
    Value(Value),

    /// More than a single value.
    Vec(Vec<Expression<Ctx, Env>>)
}

/// A manner of representing internal nodes.
pub trait Context {
    /// A representation of one or more input devices.
    type InputSet;

    /// A representation of one or more output devices.
    type OutputSet;

    /// A representation of the current state of a condition.
    type ConditionState;
}

///
/// # Launching and running the script
///


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
        let result = match self.command_sender {
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

/// A script ready to be executed.
/// Each script is meant to be executed in an individual thread.
pub struct ExecutionTask<Env> where Env: DevEnv {
    /// The current state of execution the script.
    state: Script<CompiledCtx<Env>, Env>,

    /// Communicating with the thread running script.
    tx: Sender<ExecutionOp>,
    rx: Receiver<ExecutionOp>,
}





enum ExecutionOp {
    /// An input has been updated, time to check if we have triggers
    /// ready to be executed.
    Update {index: usize, updated: DateTime<UTC>, value: Value},

    /// Time to stop executing the script.
    Stop(Box<Fn(Result<(), Error>) + Send>)
}


impl<Env> ExecutionTask<Env> where Env: ExecutableDevEnv {
    /// Create a new execution task.
    ///
    /// The caller is responsible for spawning a new thread and
    /// calling `run()`.
    fn new(script: Script<UncheckedCtx, UncheckedEnv>, tx: Sender<ExecutionOp>, rx: Receiver<ExecutionOp>) -> Result<Self, Error> {
        // Prepare the script for execution:
        // - replace instances of Input with InputDev, which map
        //   to a specific device and cache the latest known value
        //   on the input.
        // - replace instances of Output with OutputDev
        let precompiler = try!(Precompiler::new(&script));
        let bound = try!(precompiler.rebind_script(script));
        
        Ok(ExecutionTask {
            state: bound,
            rx: rx,
            tx: tx
        })
    }

    /// Execute the monitoring task.
    /// This currently expects to be executed in its own thread.
    fn run(&mut self) {
        let mut watcher = Env::get_watcher();
        let mut witnesses = Vec::new();
        
        // A thread-safe indirection towards a single input state.
        // We assume that `cells` never mutates again once we
        // have finished the loop below.
        let mut cells : Vec<Arc<CompiledInput<Env>>> = Vec::new();

        // Start listening to all inputs that appear in conditions.
        // Some inputs may appear only in expressions, so we are
        // not interested in their value.
        for rule in &self.state.rules  {
            for condition in &rule.condition.all {
                for single in &*condition.input {
                    let tx = self.tx.clone();
                    cells.push(single.clone());
                    let index = cells.len() - 1;

                    witnesses.push(
                        // We can end up watching several times the
                        // same device + capability + range.  For the
                        // moment, we do not attempt to optimize
                        // either I/O (which we expect will be
                        // optimized by `watcher`) or condition
                        // checking (which we should eventually
                        // optimize, if we find out that we end up
                        // with large rulesets).
                        watcher.add(
                            &single.device,
                            &condition.capability,
                            &condition.range,
                            move |value| {
                                // One of the inputs has been updated.
                                // Update `state` and determine
                                // whether there is anything we need
                                // to do.
                                let _ignored = tx.send(ExecutionOp::Update {
                                    updated: UTC::now(),
                                    value: value,
                                    index: index
                                });
                                // If the thread is down, it is ok to ignore messages.
                            }));
                    }
            }
        }

        // Make sure that the vector never mutates past this
        // point. This ensures that our `index` remains valid for the
        // rest of the execution.
        let cells = cells;

        // FIXME: We are going to end up with stale data in some inputs.
        // We need to find out how to get rid of it.
        // FIXME(2): We now have dates.

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

                Update {updated, value, index} => {
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
    }
}

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
                    use values::Range::*;
                    use values::Value::*;

                    match (&data.data, &self.range) {
                        // Any always matches
                        (_, &Any) => true,
                        // Operations on bools and strings
                        (&Bool(ref b), &EqBool(ref b2)) => b == b2,
                        (&String(ref s), &EqString(ref s2)) => s == s2,

                        // Numbers. FIXME: Implement physical units.
                        (&Num(ref x), &Leq(ref max)) => x <= max,
                        (&Num(ref x), &Geq(ref min)) => min <= x,
                        (&Num(ref x), &BetweenEq{ref min, ref max}) => min <= x && x <= max,
                        (&Num(ref x), &OutOfStrict{ref min, ref max}) => x < min || max < x,

                        // Type errors don't match.
                        (&Bool(_), _) => false,
                        (&String(_), _) => false,
                        (_, &EqBool(_)) => false,
                        (_, &EqString(_)) => false,
                        (&Vec(_), _) => false,

                        // There is no such thing as a range on json or blob.
                        (&Json(_), _) |
                        (&Blob{..}, _) => false,
                    }
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

impl<Env> Expression<CompiledCtx<Env>, Env> where Env: ExecutableDevEnv {
    fn eval(&self) -> Value {
        match *self {
            Expression::Value(ref v) => v.clone(),
            Expression::Input(_) => panic!("Cannot read an input in an expression yet"),
            Expression::Vec(ref vec) => {
                Value::Vec(vec.iter().map(|expr| expr.eval()).collect())
            }
        }
    }
}


#[derive(Debug)]
pub enum SourceError {
    AllocationLengthError { allocations: usize, requirements: usize},
    NoCapability, // FIXME: Add details
    NoSuchInput, // FIXME: Add details
    NoSuchOutput, // FIXME: Add details
}

#[derive(Debug)]
pub enum DevAccessError {
    DeviceNotFound, // FIXME: Add details
    DeviceKindNotFound, // FIXME: Add details
    DeviceCapabilityNotFound, // FIXME: Add details
    CouldNotSend, // FIXME: Add details
}

#[derive(Debug)]
pub enum RunningError {
    AlreadyRunning,
    NotRunning,
}

#[derive(Debug)]
pub enum Error {
    SourceError(SourceError),
    DevAccessError(DevAccessError),
    RunningError(RunningError),
}




///
/// # Precompilation
///

/// A Context used to represent a script that hasn't been compiled
/// yet. Rather than pointing to specific device + capability, inputs
/// and outputs are numbers that are meaningful only in the AST.
pub struct UncheckedCtx;
impl Context for UncheckedCtx {
    /// In this implementation, each input is represented by its index
    /// in the array of allocations.
    type InputSet = usize;

    /// In this implementation, each output is represented by its
    /// index in the array of allocations.
    type OutputSet = usize;

    /// In this implementation, conditions have no state.
    type ConditionState = ();
}

/// A DevEnv used to represent a script that hasn't been
/// compiled yet. Rather than having typed devices, capabilities,
/// etc. everything is represented by a string.
pub struct UncheckedEnv;
impl DevEnv for UncheckedEnv {
    type Device = String;
    type DeviceKind = String;
    type InputCapability = String;
    type OutputCapability = String;
}

struct CompiledCtx<DevEnv> {
    phantom: PhantomData<DevEnv>,
}

struct CompiledInput<Env> where Env: DevEnv {
    device: Env::Device,
    state: RwLock<Option<DatedData>>,
}

struct CompiledOutput<Env> where Env: DevEnv {
    device: Env::Device,
}

type CompiledInputSet<Env> = Vec<Arc<CompiledInput<Env>>>;
type CompiledOutputSet<Env> = Vec<Arc<CompiledOutput<Env>>>;
struct CompiledConditionState {
    is_met: bool
}

impl<Env> Context for CompiledCtx<Env> where Env: DevEnv {
    type ConditionState = CompiledConditionState; // FIXME: We could share this
    type OutputSet = CompiledOutputSet<Env>;
    type InputSet = CompiledInputSet<Env>;
}


pub struct FakeWatcher;
impl Watcher for FakeWatcher {
    type Witness = ();
    type Device = String;
    type InputCapability = String;

    fn add<F>(&mut self,
              device: &Self::Device,
              input: &Self::InputCapability,
              condition: &Range,
              cb: F) -> Self::Witness where F:Fn(Value)
    {
        panic!("Cannot execute a FakeWatcher");
    }
}

/// Data, labelled with its latest update.
struct DatedData {
    updated: DateTime<UTC>,
    data: Value,
}

struct Precompiler<Env> where Env: ExecutableDevEnv {
    inputs: Vec<Option<CompiledInputSet<Env>>>,
    outputs: Vec<Option<CompiledOutputSet<Env>>>,
    phantom: PhantomData<Env>,
}

impl<Env> Precompiler<Env> where Env: ExecutableDevEnv {
    fn new(source: &Script<UncheckedCtx, UncheckedEnv>) -> Result<Self, Error> {

        use self::Error::*;
        use self::SourceError::*;
        use self::DevAccessError::*;

        // In an UncheckedCtx, inputs and outputs are (unchecked)
        // indices towards the vector of allocations. In this step,
        // we 1/ check the indices, to make sure that they actually
        // point inside the vector;
        // 2/ prepare arrays `inputs` and `outputs`, which will later
        // serve to replace the indices by pointers to the Arc containing
        // details on the device and its state.

        let mut inputs = Vec::new();
        let mut outputs = Vec::new();

        if source.allocations.len() != source.requirements.len() {
            return Err(SourceError(AllocationLengthError {
                allocations: source.allocations.len(),
                requirements: source.requirements.len()
            }));
        }

        for (alloc, req) in source.allocations.iter().zip(&source.requirements) {
            let mut input = None;
            let mut output = None;

            let has_inputs = req.inputs.len() > 0;
            let has_outputs = req.outputs.len() > 0;
            if  !has_inputs && !has_outputs {
                // An empty resource? This doesn't make sense.
                return Err(SourceError(NoCapability));
            }

            if has_inputs {
                let mut resolved = Vec::with_capacity(alloc.devices.len());
                for dev in &alloc.devices {
                    match Env::get_device(&dev) {
                        None => return Err(DevAccessError(DeviceNotFound)),
                        Some(d) => resolved.push(Arc::new(CompiledInput {
                            device: d,
                            state: RwLock::new(None)
                        }))
                    }
                }
                input = Some(resolved);
            }
            if has_outputs {
                let mut resolved = Vec::with_capacity(alloc.devices.len());
                for dev in &alloc.devices {
                    match Env::get_device(&dev) {
                        None => return Err(DevAccessError(DeviceNotFound)),
                        Some(d) => resolved.push(Arc::new(CompiledOutput {
                            device: d,
                        }))
                    }
                }
                output = Some(resolved);
            }
            inputs.push(input);
            outputs.push(output);
        }

        Ok(Precompiler {
            inputs: inputs,
            outputs: outputs,
            phantom: PhantomData
        })
    }
}

impl<Env> Precompiler<Env>
    where Env: ExecutableDevEnv {

    fn rebind_script(&self, script: Script<UncheckedCtx, UncheckedEnv>) -> Result<Script<CompiledCtx<Env>, Env>, Error>
    {
        let rules = try!(map(script.rules, |rule| self.rebind_trigger(rule)));

        let allocations = try!(map(script.allocations, |res| {
            let devices = try!(map(res.devices, |dev| {
                self.rebind_device(dev)
            }));
            Ok(Resource {
                devices: devices,
                phantom: PhantomData,
            })
        }));

        let requirements = try!(map(script.requirements, |req| {
            let inputs = try!(map(req.inputs, |input| {
                self.rebind_input_capability(input)
            }));
            let outputs = try!(map(req.outputs, |output| {
                self.rebind_output_capability(output)
            }));
            Ok(Requirement {
                kind: try!(self.rebind_device_kind(req.kind)),
                inputs: inputs,
                outputs: outputs,
                phantom: PhantomData
            })
        }));

        Ok(Script {
            metadata: (),
            requirements: requirements,
            allocations: allocations,
            rules: rules
        })
    }

    fn rebind_trigger(&self, trigger: Trigger<UncheckedCtx, UncheckedEnv>) -> Result<Trigger<CompiledCtx<Env>, Env>, Error>
    {
        let execute = try!(map(trigger.execute, |statement| {
            self.rebind_statement(statement)
        }));
        Ok(Trigger {
            execute: execute,
            condition: try!(self.rebind_conjunction(trigger.condition))
        })
    }

    fn rebind_conjunction(&self, conjunction: Conjunction<UncheckedCtx, UncheckedEnv>) -> Result<Conjunction<CompiledCtx<Env>, Env>, Error>
    {
        let all = try!(map(conjunction.all, |condition| {
            self.rebind_condition(condition)
        }));
        Ok(Conjunction {
            all: all,
            state: try!(self.rebind_condition_state(conjunction.state))
        })
    }

    fn rebind_condition(&self, condition: Condition<UncheckedCtx, UncheckedEnv>) -> Result<Condition<CompiledCtx<Env>, Env>, Error>
    {
        Ok(Condition {
            range: condition.range,
            capability: try!(self.rebind_input_capability(condition.capability)),
            input: try!(self.rebind_input(condition.input)),
            state: try!(self.rebind_condition_state(condition.state))
        })
    }

    fn rebind_statement(&self, statement: Statement<UncheckedCtx, UncheckedEnv>) -> Result<Statement<CompiledCtx<Env>, Env>, Error>
    {
        let mut arguments = HashMap::with_capacity(statement.arguments.len());
        for (key, expr) in statement.arguments {
            arguments.insert(key.clone(), try!(self.rebind_expression(expr)));
        }
        Ok(Statement {
            destination: try!(self.rebind_output(statement.destination)),
            action: try!(self.rebind_output_capability(statement.action)),
            arguments: arguments
        })
    }

    fn rebind_expression(&self, expression: Expression<UncheckedCtx, UncheckedEnv>) -> Result<Expression<CompiledCtx<Env>, Env>, Error>
    {
        let expression = match expression {
            Expression::Value(v) => Expression::Value(v),
            Expression::Vec(v) => {
                Expression::Vec(try!(map(v, |expr| {
                    self.rebind_expression(expr)
                })))
            }
            Expression::Input(_) => panic!("Not implemented yet")
        };
        Ok(expression)
    }

    fn rebind_device(&self, dev: <UncheckedEnv as DevEnv>::Device) -> Result<Env::Device, Error>
    {
        match Env::get_device(&dev) {
            None => Err(Error::DevAccessError(DevAccessError::DeviceNotFound)),
            Some(found) => Ok(found.clone())
        }
    }


    fn rebind_device_kind(&self, kind: <UncheckedEnv as DevEnv>::DeviceKind) ->
        Result<Env::DeviceKind, Error>
    {
        match Env::get_device_kind(&kind) {
            None => Err(Error::DevAccessError(DevAccessError::DeviceKindNotFound)),
            Some(found) => Ok(found.clone())
        }
    }
    
    fn rebind_input_capability(&self, cap: <UncheckedEnv as DevEnv>::InputCapability) ->
        Result<Env::InputCapability, Error>
    {
        match Env::get_input_capability(&cap) {
            None => Err(Error::DevAccessError(DevAccessError::DeviceCapabilityNotFound)),
            Some(found) => Ok(found.clone())
        }
    }

    fn rebind_output_capability(&self, cap: <UncheckedEnv as DevEnv>::OutputCapability) ->
        Result<Env::OutputCapability, Error>
    {
        match Env::get_output_capability(&cap) {
            None => Err(Error::DevAccessError(DevAccessError::DeviceCapabilityNotFound)),
            Some(found) => Ok(found.clone())
        }
    }

    // Rebinding the context
    fn rebind_condition_state(&self, state: <UncheckedCtx as Context>::ConditionState) ->
        Result<<CompiledCtx<Env> as Context>::ConditionState, Error>
    {
        // By default, conditions are not met.
        Ok(CompiledConditionState {
            is_met: false
        })
    }

    fn rebind_input(&self, index: <UncheckedCtx as Context>::InputSet) ->
        Result<<CompiledCtx<Env> as Context>::InputSet, Error>
    {
        match self.inputs[index] {
            None => Err(Error::SourceError(SourceError::NoSuchInput)),
            Some(ref input) => Ok(input.clone())
        }
    }


    fn rebind_output(&self, index: <UncheckedCtx as Context>::OutputSet) ->
        Result<<CompiledCtx<Env> as Context>::OutputSet, Error>
    {
        match self.outputs[index] {
            None => Err(Error::SourceError(SourceError::NoSuchOutput)),
            Some(ref output) => Ok(output.clone())
        }
    }
}


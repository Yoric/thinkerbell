
#![allow(unused_variables)]
#![allow(dead_code)]

/// Basic structure of a Monitor (aka Server App, aka wtttttt)
///
/// Monitors are designed so that the FoxBox can offer a simple
/// IFTTT-style Web UX to let users write their own scripts. More
/// complex monitors can installed from the web from a master device
/// (i.e. the user's cellphone or smart tv).

use dependencies::{DeviceAccess, OutputCapability, Range, Value, Watcher};

use std::collections::HashMap;
use std::sync::{Arc, RwLock}; // FIXME: Investigate if we really need so many instances of Arc. I suspect that most can be replaced by &'a.
use std::sync::mpsc::{channel, Receiver, Sender};
use std::marker::PhantomData;
use std::result::Result;
use std::result::Result::*;

extern crate chrono;
use self::chrono::{Duration, DateTime, UTC};

extern crate rustc_serialize;
use self::rustc_serialize::json::Json;


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
pub struct Script<Ctx, Dev> where Dev: DeviceAccess, Ctx: Context {
    /// Authorization, author, description, update url, version, ...
    metadata: (), // FIXME: Implement

    /// Monitor applications have sets of requirements (e.g. "I need a
    /// camera"), which are allocated to actual resources through the
    /// UX. Re-allocating resources may be requested by the user, the
    /// foxbox, or an application, e.g. when replacing a device or
    /// upgrading the app.
    requirements: Vec<Arc<Requirement<Ctx, Dev>>>,

    /// Resources actually allocated for each requirement.
    /// This must have the same size as `requirements`.
    allocations: Vec<Resource<Ctx, Dev>>,

    /// A set of rules, stating what must be done in which circumstance.
    rules: Vec<Trigger<Ctx, Dev>>,
}

struct Resource<Ctx, Dev> where Dev: DeviceAccess, Ctx: Context {
    devices: Vec<Dev::Device>,
    phantom: PhantomData<Ctx>,
}


/// A resource needed by this application. Typically, a definition of
/// device with some input our output capabilities.
struct Requirement<Ctx, Dev> where Dev: DeviceAccess, Ctx: Context {
    /// The kind of resource, e.g. "a flashbulb".
    kind: Dev::DeviceKind,

    /// Input capabilities we need from the device, e.g. "the time of
    /// day", "the current temperature".
    inputs: Vec<Dev::InputCapability>,

    /// Output capabilities we need from the device, e.g. "play a
    /// sound", "set luminosity".
    outputs: Vec<Dev::OutputCapability>,
    
    /// Minimal number of resources required. If unspecified in the
    /// script, this is 1.
    min: u32,

    /// Maximal number of resources that may be handled. If
    /// unspecified in the script, this is the same as `min`.
    max: u32,

    phantom: PhantomData<Ctx>,
    // FIXME: We may need cooldown properties.
}

/// A single trigger, i.e. "when some condition becomes true, do
/// something".
struct Trigger<Ctx, Dev> where Dev: DeviceAccess, Ctx: Context {
    /// The condition in which to execute the trigger.
    condition: Conjunction<Ctx, Dev>,

    /// Stuff to do once `condition` is met.
    execute: Vec<Statement<Ctx, Dev>>,

    /// Minimal duration between two executions of the trigger.  If a
    /// duration was not picked by the developer, a reasonable default
    /// duration should be picked (e.g. 10 minutes).
    cooldown: Duration,
}

/// A conjunction (e.g. a "and") of conditions.
struct Conjunction<Ctx, Dev> where Dev: DeviceAccess, Ctx: Context {
    /// The conjunction is true iff all of the following expressions evaluate to true.
    all: Vec<Condition<Ctx, Dev>>,
    state: Ctx::ConditionState,
}

/// An individual condition.
///
/// Conditions always take the form: "data received from sensor is in
/// given range".
///
/// A condition is true if *any* of the sensors allocated to this
/// requirement has yielded a value that is in the given range.
struct Condition<Ctx, Dev> where Dev: DeviceAccess, Ctx: Context {
    input: Ctx::InputSet,
    capability: Dev::InputCapability,
    range: Range,
    state: Ctx::ConditionState,
}


/// Stuff to actually do. In practice, this means placing calls to devices.
struct Statement<Ctx, Dev> where Dev: DeviceAccess, Ctx: Context {
    /// The resource to which this command applies.  e.g. "all
    /// heaters", "a single communication channel", etc.
    destination: Ctx::OutputSet,

    /// The action to execute on the resource.
    action: Dev::OutputCapability,

    /// Data to send to the resource.
    arguments: HashMap<String, Expression<Ctx, Dev>>
}

struct InputSet<Ctx, Dev> where Dev: DeviceAccess, Ctx: Context {
    /// The set of inputs from which to grab the value.
    condition: Condition<Ctx, Dev>,
    /// The value to grab.
    capability: Dev::InputCapability,
}

/// A value that may be sent to an output.
enum Expression<Ctx, Dev> where Dev: DeviceAccess, Ctx: Context {
    /// A dynamic value, which must be read from one or more inputs.
    // FIXME: Not ready yet
    Input(InputSet<Ctx, Dev>),

    /// A constant value.
    Value(Value),

    /// More than a single value.
    Vec(Vec<Expression<Ctx, Dev>>)
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

/// A script ready to be executed.
/// Each script is meant to be executed in an individual thread.
struct ExecutionTask<Ctx, Dev> where Dev: DeviceAccess, Ctx: Context {
    /// The current state of execution the script.
    state: Script<Ctx, Dev>,

    /// Communicating with the thread running script.
    tx: Sender<ExecutionOp>,
    rx: Receiver<ExecutionOp>,
}



struct IsMet {
    old: bool,
    new: bool,
}


enum ExecutionOp {
    /// An input has been updated, time to check if we have triggers
    /// ready to be executed.
    Update,

    /// Time to stop executing the script.
    Stop
}


impl<Ctx, Dev> ExecutionTask<Ctx, Dev> where Dev: DeviceAccess, Ctx: Context {
    /// Create a new execution task.
    ///
    /// The caller is responsible for spawning a new thread and
    /// calling `run()`.
    fn new(script: &Script<UncheckedCtx, UncheckedDev>) -> Self {
        panic!("Not implemented");
/*
        // Prepare the script for execution:
        // - replace instances of Input with InputDev, which map
        //   to a specific device and cache the latest known value
        //   on the input.
        // - replace instances of Output with OutputDev
        let precompiler = Precompiler::new(script);
        let bound = script.rebind(&precompiler);
        
        let (tx, rx) = channel();
        ExecutionTask {
            state: bound,
            rx: rx,
            tx: tx
        }
*/
    }

    /// Get a channel that may be used to send commands to the task.
    fn get_command_sender(&self) -> Sender<ExecutionOp> {
        self.tx.clone()
    }

    /// Execute the monitoring task.
    /// This currently expects to be executed in its own thread.
    fn run(&mut self) {
        panic!("Not implemented");
        /*
        let mut watcher = Dev::Watcher::new();
        let mut witnesses = Vec::new();

        // Start listening to all inputs that appear in conditions.
        // Some inputs may appear only in expressions, so we are
        // not interested in their value.
        for rule in &self.state.rules  {
            for condition in &rule.condition.all {
                for single in &*condition.input {
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
                            |value| {
                                // One of the inputs has been updated.
                                *single.state.write().unwrap() = Some(DatedData {
                                    updated: UTC::now(),
                                    data: value
                                });
                                // Note that we can unwrap() safely,
                                // as it fails only if the thread is
                                // already in panic.

                                // Find out if we should execute one of the
                                // statements of the trigger.
                                let _ignored = self.tx.send(ExecutionOp::Update);
                                // If the thread is down, it is ok to ignore messages.
                            }));
                    }
            }
        }

        // FIXME: We are going to end up with stale data in some inputs.
        // We need to find out how to get rid of it.
        // FIXME(2): We now have dates.

        // Now, start handling events.
        for msg in &self.rx {
            use self::ExecutionOp::*;
            match msg {
                Stop => {
                    // Leave the loop.
                    // The watcher and the witnesses will be cleaned up on exit.
                    // Any further message will be ignored.
                    return;
                }

                Update => {
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
                            // FIXME: Execute
                        }
                    }
                }
            }
        }
*/
    }
}

///
/// # Evaluating conditions
///
/*
impl<Ctx, Dev> Trigger<Ctx, Dev> where Dev: DeviceAccess, Ctx: Context {
    fn is_met(&mut self) -> IsMet {
        self.condition.is_met()
    }
}

impl<Ctx, Dev> Conjunction<Ctx, Dev> where Dev: DeviceAccess, Ctx: Context {
    /// For a conjunction to be true, all its components must be true.
    fn is_met(&mut self) -> IsMet {
        let &mut is_met = Dev::condition_is_met(&mut self.state);
        let old = is_met;
        let mut new = true;

        for mut single in &mut self.all {
            if !single.is_met().new {
                new = false;
                // Don't break. We want to make sure that we update
                // `is_met` of all individual conditions.
            }
        }
        is_met = new;
        IsMet {
            old: old,
            new: new,
        }
    }
}

impl<Ctx, Dev> Condition<Ctx, Dev> where Dev: DeviceAccess, Ctx: Context {
    /// Determine if one of the devices serving as input for this
    /// condition meets the condition.
    fn is_met(&mut self) -> IsMet {
        let &mut is_met = Dev::condition_is_met(&mut self.state);
        let old = is_met;
        let mut new = false;
        for single in Dev::get_inputs(&mut self.input) {
            // This will fail only if the thread has already panicked.
            let state = single.state.read().unwrap();
            let is_met = match *state {
                None => { false /* We haven't received a measurement yet.*/ },
                Some(ref data) => {
                    use dependencies::Range::*;
                    use dependencies::Value::*;

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
*/

pub enum Error { // FIXME: Add details
    CompilationError
}

/// Rebind a script from an environment to another one.
///
/// This is typically used as a compilation step, to turn code in
/// which device kinds, device allocations, etc. are represented as
/// strings or numbers into code in which they are represented by
/// concrete data structures.
trait Rebinder {
    type SourceCtx: Context;
    type DestCtx: Context;
    type SourceDev: DeviceAccess;
    type DestDev: DeviceAccess;

    // Rebinding the device access
    fn rebind_device(&self, &<<Self as Rebinder>::SourceDev as DeviceAccess>::Device) ->
        Result<<<Self as Rebinder>::DestDev as DeviceAccess>::Device, Error>;
    fn rebind_device_kind(&self, &<<Self as Rebinder>::SourceDev as DeviceAccess>::DeviceKind) ->
        Result<<<Self as Rebinder>::DestDev as DeviceAccess>::DeviceKind, Error>;
    fn rebind_input_capability(&self, &<<Self as Rebinder>::SourceDev as DeviceAccess>::InputCapability) ->
        Result<<<Self as Rebinder>::DestDev as DeviceAccess>::InputCapability, Error>;
    fn rebind_output_capability(&self, &<<Self as Rebinder>::SourceDev as DeviceAccess>::OutputCapability) ->
        Result<<<Self as Rebinder>::DestDev as DeviceAccess>::OutputCapability, Error>;

    // Rebinding the context
    fn rebind_input(&self, &<<Self as Rebinder>::SourceCtx as Context>::InputSet) ->
        Result<<<Self as Rebinder>::DestCtx as Context>::InputSet, Error>;

    fn rebind_output(&self, &<<Self as Rebinder>::SourceCtx as Context>::OutputSet) ->
        Result<<<Self as Rebinder>::DestCtx as Context>::OutputSet, Error>;

    fn rebind_condition(&self, &<<Self as Rebinder>::SourceCtx as Context>::ConditionState) ->
        Result<<<Self as Rebinder>::DestCtx as Context>::ConditionState, Error>;
}

impl<Ctx, Dev> Script<Ctx, Dev> where Dev: DeviceAccess, Ctx: Context {
    fn rebind<R>(&self, rebinder: &R) -> Result<Script<R::DestCtx, R::DestDev>, Error>
        where R: Rebinder<SourceDev = Dev, SourceCtx = Ctx>
    {
        let mut rules = Vec::with_capacity(self.rules.len());
        for rule in &self.rules {
            rules.push(try!(rule.rebind(rebinder)));
        }

        let mut allocations = Vec::with_capacity(self.allocations.len());
        for res in &self.allocations {
            let mut devices = Vec::with_capacity(res.devices.len());
            for dev in &res.devices {
                devices.push(try!(rebinder.rebind_device(&dev)));
            }
            allocations.push(Resource {
                devices: devices,
                phantom: PhantomData,
            });
        }

        let mut requirements = Vec::with_capacity(self.requirements.len());
        for req in &self.requirements {
            let mut inputs = Vec::with_capacity(req.inputs.len());
            for cap in &req.inputs {
                inputs.push(try!(rebinder.rebind_input_capability(cap)));
            }

            let mut outputs = Vec::with_capacity(req.outputs.len());
            for cap in &req.outputs {
                outputs.push(try!(rebinder.rebind_output_capability(cap)));
            }

            requirements.push(Arc::new(Requirement {
                kind: try!(rebinder.rebind_device_kind(&req.kind)),
                inputs: inputs,
                outputs: outputs,
                min: req.min,
                max: req.max,
                phantom: PhantomData,
            }));
        }

        Ok(Script {
            metadata: self.metadata.clone(),
            requirements: requirements,
            allocations: allocations,
            rules: rules,
        })
    }
}


impl<Ctx, Dev> Trigger<Ctx, Dev> where Dev: DeviceAccess, Ctx: Context {
    fn rebind<R>(&self, rebinder: &R) -> Result<Trigger<R::DestCtx, R::DestDev>, Error>
        where R: Rebinder<SourceDev = Dev, SourceCtx = Ctx>
    {
        let mut execute = Vec::with_capacity(self.execute.len());
        for ex in &self.execute {
            execute.push(try!(ex.rebind(rebinder)));
        }
        Ok(Trigger {
            cooldown: self.cooldown.clone(),
            execute: execute,
            condition: try!(self.condition.rebind(rebinder)),
        })
    }
}

impl<Ctx, Dev> Conjunction<Ctx, Dev> where Dev: DeviceAccess, Ctx: Context {
    fn rebind<R>(&self, rebinder: &R) -> Result<Conjunction<R::DestCtx, R::DestDev>, Error>
        where R: Rebinder<SourceDev = Dev, SourceCtx = Ctx>
    {
        let mut all = Vec::with_capacity(self.all.len());
        for c in &self.all {
            all.push(try!(c.rebind(rebinder)));
        }
        Ok(Conjunction {
            all: all,
            state: try!(rebinder.rebind_condition(&self.state)),
        })
    }
}


impl<Ctx, Dev> Condition<Ctx, Dev> where Dev: DeviceAccess, Ctx: Context {
    fn rebind<R>(&self, rebinder: &R) -> Result<Condition<R::DestCtx, R::DestDev>, Error>
        where R: Rebinder<SourceDev = Dev, SourceCtx = Ctx>
    {
        Ok(Condition {
            range: self.range.clone(),
            capability: try!(rebinder.rebind_input_capability(&self.capability)),
            input: try!(rebinder.rebind_input(&self.input)),
            state: try!(rebinder.rebind_condition(&self.state)),
        })
    }
}



impl<Ctx, Dev> Statement<Ctx, Dev> where Dev: DeviceAccess, Ctx: Context {
    fn rebind<R>(&self, rebinder: &R) -> Result<Statement<R::DestCtx, R::DestDev>, Error>
        where R: Rebinder<SourceDev = Dev, SourceCtx = Ctx>
    {
        let mut arguments = HashMap::with_capacity(self.arguments.len());
        for (key, value) in &self.arguments {
            arguments.insert(key.clone(), try!(value.rebind(rebinder)));
        }
        Ok(Statement {
            destination: try!(rebinder.rebind_output(&self.destination)),
            action: try!(rebinder.rebind_output_capability(&self.action)),
            arguments: arguments
        })
    }
}

impl<Ctx, Dev> Expression<Ctx, Dev> where Dev: DeviceAccess, Ctx: Context {
    fn rebind<R>(&self, rebinder: &R) -> Result<Expression<R::DestCtx, R::DestDev>, Error>
        where R: Rebinder<SourceDev = Dev, SourceCtx = Ctx>
    {
        match *self {
            Expression::Value(ref v) => Ok(Expression::Value(v.clone())),
            Expression::Vec(ref v) => {
                let mut v2 = Vec::with_capacity(v.len());
                for x in v {
                    v2.push(try!(x.rebind(rebinder)));
                }
                Ok(Expression::Vec(v2))
            }
            //            Input(ref input) => Input(rebinder.rebind_input(input).clone()),
            Expression::Input(_) => panic!("Not impl implemented yet")
        }
    }
}


///
/// # Precompilation
///

/// A Context used to represent a script that hasn't been compiled
/// yet. Rather than pointing to specific device + capability, inputs
/// and outputs are numbers that are meaningful only in the AST.
struct UncheckedCtx;
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

/// A DeviceAccess used to represent a script that hasn't been
/// compiled yet. Rather than having typed devices, capabilities,
/// etc. everything is represented by a string.
struct UncheckedDev;
impl DeviceAccess for UncheckedDev {
    type Device = String;
    type DeviceKind = String;
    type InputCapability = String;
    type OutputCapability = String;
    type Watcher = FakeWatcher;

    fn get_device_kind(&self, key: &String) -> Option<String> {
        Some(key.clone())
    }

    fn get_device(&self, key: &String) -> Option<String> {
        Some(key.clone())
    }

    fn get_input_capability(&self, key: &String) -> Option<String> {
        Some(key.clone())
    }

    fn get_output_capability(&self, key: &String) -> Option<String> {
        Some(key.clone())
    }
}

struct CompiledCtx<DeviceAccess> {
    phantom: PhantomData<DeviceAccess>,
}

struct CompiledInput<Dev> where Dev: DeviceAccess {
    device: Dev::Device,
    state: RwLock<Option<DatedData>>,
}

struct CompiledOutput<Dev> where Dev: DeviceAccess {
    device: Dev::Device,
}

type CompiledInputSet<Dev> = Arc<Vec<Arc<CompiledInput<Dev>>>>;
type CompiledOutputSet<Dev> = Arc<Vec<Arc<CompiledOutput<Dev>>>>;

impl<Dev> Context for CompiledCtx<Dev> where Dev: DeviceAccess {
    type ConditionState = bool;
    type OutputSet = CompiledOutputSet<Dev>;
    type InputSet = CompiledInputSet<Dev>;
}


struct FakeWatcher;
impl Watcher for FakeWatcher {
    type Witness = ();
    type Device = String;
    type InputCapability = String;

    fn new() -> FakeWatcher {
        panic!("Cannot instantiate a FakeWatcher");
    }

    fn add<F>(&mut self,
              device: &Self::Device,
              input: &Self::InputCapability,
              condition: &Range,
              cb: F) -> () where F:FnOnce(Value)
    {
        panic!("Cannot execute a FakeWatcher");
    }
}

/*
impl DeviceAccess for CompiledDev {
    type Input = Vector<Arc<InputDev>>;
    type Output = Vector<Arc<OutputDev>>;
    type ConditionState = ConditionDev;
    type Device = Device;
    type InputCapability = InputCapability;
    type OutputCapability = OutputCapability;
}

impl ExecutionDeviceAccess for CompiledDev {
    type Watcher = Box<Watcher>;
    fn condition_is_met<'a>(is_met: &'a mut Self::ConditionState) -> &'a IsMet {
        is_met
    }
}
 */

/// Data, labelled with its latest update.
struct DatedData {
    updated: DateTime<UTC>,
    data: Value,
}

struct Precompiler<'a, Dev> where Dev: DeviceAccess {
    script: &'a Script<UncheckedCtx, UncheckedDev>,
    inputs: Vec<Option<CompiledInputSet<Dev>>>,
    outputs: Vec<Option<CompiledOutputSet<Dev>>>,
    phantom: PhantomData<Dev>,
}

impl<'a, Dev> Precompiler<'a, Dev> where Dev: DeviceAccess {
    fn new(source: &'a Script<UncheckedCtx, UncheckedDev>) -> Result<Self, ()> {
        // Precompute allocations
        let inputs = Vec::new();
        let outputs = Vec::new();

        // FIXME: Populate

        Ok(Precompiler {
            script: source,
            inputs: inputs,
            outputs: outputs,
            phantom: PhantomData
        })
    }
}

impl<'a, Dev> Rebinder for Precompiler<'a, Dev>
    where Dev: DeviceAccess {
    type SourceCtx = UncheckedCtx;
    type DestCtx = CompiledCtx<Dev>;

    type SourceDev = Dev;
    type DestDev = Dev;

    // Rebinding the device access. Nothing to do.
    fn rebind_device(&self, dev: &<<Self as Rebinder>::SourceDev as DeviceAccess>::Device) ->
        Result<<<Self as Rebinder>::DestDev as DeviceAccess>::Device, Error>
    {
        Ok(dev.clone())
    }


    fn rebind_device_kind(&self, kind: &<<Self as Rebinder>::SourceDev as DeviceAccess>::DeviceKind) ->
        Result<<<Self as Rebinder>::DestDev as DeviceAccess>::DeviceKind, Error>
    {
        Ok((*kind).clone())
    }
    
    fn rebind_input_capability(&self, cap: &<<Self as Rebinder>::SourceDev as DeviceAccess>::InputCapability) ->
        Result<<<Self as Rebinder>::DestDev as DeviceAccess>::InputCapability, Error>
    {
        Ok((*cap).clone())
    }

    fn rebind_output_capability(&self, cap: &<<Self as Rebinder>::SourceDev as DeviceAccess>::OutputCapability) ->
        Result<<<Self as Rebinder>::DestDev as DeviceAccess>::OutputCapability, Error>
    {
        Ok((*cap).clone())
    }

    // Recinding the context
    fn rebind_condition(&self, state: &<<Self as Rebinder>::SourceCtx as Context>::ConditionState) ->
        Result<<<Self as Rebinder>::DestCtx as Context>::ConditionState, Error>
    {
        // By default, conditions are not met.
        Ok(false)
    }

    fn rebind_input(&self, index: &<<Self as Rebinder>::SourceCtx as Context>::InputSet) ->
        Result<<<Self as Rebinder>::DestCtx as Context>::InputSet, Error>
    {
        match self.inputs[*index] {
            None => Err(Error::CompilationError),
            Some(ref input) => Ok(input.clone())
        }
    }


    fn rebind_output(&self, index: &<<Self as Rebinder>::SourceCtx as Context>::OutputSet) ->
        Result<<<Self as Rebinder>::DestCtx as Context>::OutputSet, Error>
    {
        match self.outputs[*index] {
            None => Err(Error::CompilationError),
            Some(ref output) => Ok(output.clone())
        }
    }
}
/*
impl Script {
    ///
    /// Start executing the application.
    ///
    pub fn start(&mut self) {
        if self.command_sender.is_some() {
            return;
        }
        let mut task = MonitorTask::new(self.clone());
        self.command_sender = Some(task.get_command_sender());
        thread::spawn(move || {
            task.run();
        });
    }

    ///
    /// Stop the execution of the application.
    ///
    pub fn stop(&mut self) {
        match self.command_sender {
            None => {
                /* Nothing to stop */
                return;
            },
            Some(ref tx) => {
                // Shutdown the application, asynchronously.
                let _ignored = tx.send(MonitorOp::Stop);
                // Do not return.
            }
        }
        self.command_sender = None;
    }
}

*/

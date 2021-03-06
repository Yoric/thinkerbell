use compile::ExecutableDevEnv;

use foxbox_taxonomy::api::{ API, Error, User };
use foxbox_taxonomy::manager::*;
use foxbox_taxonomy::services::*;
use foxbox_taxonomy::values::*;

use std::cmp::{ Ord, PartialOrd, Ordering as OrdOrdering };
use std::fmt;
use std::collections::{ BinaryHeap, HashMap };
use std::sync::{ Arc, Mutex };
use std::sync::atomic::{ AtomicBool, Ordering as AtomicOrdering };
use std::thread;

use transformable_channels::mpsc::*;

use chrono::{ DateTime, UTC };

use serde::ser::{Serialize, Serializer};
use serde::de::{Deserialize, Deserializer};



static VERSION : [u32;4] = [0, 0, 0, 0];

/// A back-end holding values and watchers, shared by all the virtual adapters of this simulator.
struct TestSharedAdapterBackend {
    /// The latest known value for each getter.
    getter_values: HashMap<Id<Getter>, Result<Value, Error>>,
    setter_errors: HashMap<Id<Setter>, Error>,

    /// A channel to inform when things happen.
    on_event: Box<ExtSender<FakeEnvEvent>>,

    /// All watchers. 100% counter-optimized
    counter: usize,

    timers: BinaryHeap<Timer>,
    trigger_timers_until: Option<DateTime<UTC>>,

    watchers: HashMap<usize, (Id<Getter>, Option<Range>, Box<ExtSender<WatchEvent>>)>,
}

impl TestSharedAdapterBackend {
    fn new(on_event: Box<ExtSender<FakeEnvEvent>>) -> Self {
        TestSharedAdapterBackend {
            getter_values: HashMap::new(),
            setter_errors: HashMap::new(),
            on_event: on_event,
            counter: 0,
            watchers: HashMap::new(),
            timers: BinaryHeap::new(),
            trigger_timers_until: None,
        }
    }

    fn fetch_values(&self, mut getters: Vec<Id<Getter>>, _: User) -> ResultMap<Id<Getter>, Option<Value>, Error> {
        getters.drain(..).map(|id| {
            let got = self.getter_values.get(&id).cloned();
            match got {
                None => (id, Ok(None)),
                Some(Ok(value)) => (id, Ok(Some(value))),
                Some(Err(err)) => (id, Err(err)),
            }
        }).collect()
    }

    fn send_values(&self, mut values: HashMap<Id<Setter>, Value>, _: User) -> ResultMap<Id<Setter>, (), Error> {
        values.drain().map(|(id, value)| {
            match self.setter_errors.get(&id) {
                None => {
                    let event = FakeEnvEvent::Send {
                        id: id.clone(),
                        value: value
                    };
                    let _ = self.on_event.send(event);
                    (id, Ok(()))
                }
                Some(error) => {
                    (id, Err(error.clone()))
                }
            }
        }).collect()
    }

    fn register_watch(&mut self, mut source: Vec<(Id<Getter>, Option<Range>, Box<ExtSender<WatchEvent>>)>) ->
            Vec<(Id<Getter>, Result<usize, Error>)>
    {
        let results = source.drain(..).map(|(id, range, tx)| {
            let result = (id.clone(), Ok(self.counter));
            self.watchers.insert(self.counter, (id, range, tx));
            self.counter += 1;
            result
        }).collect();
        results
    }

    fn remove_watch(&mut self, key: usize) {
        let _ = self.watchers.remove(&key);
    }

    fn inject_setter_errors(&mut self, mut errors: Vec<(Id<Setter>, Option<Error>)>)
    {
        for (id, error) in errors.drain(..) {
            match error {
                None => {
                    self.setter_errors.remove(&id);
                },
                Some(error) => {
                    self.setter_errors.insert(id, error);
                }
            }
        }
    }

    fn inject_getter_values(&mut self, mut values: Vec<(Id<Getter>, Result<Value, Error>)>)
    {
        for (id, value) in values.drain(..) {
            let old = self.getter_values.insert(id.clone(), value.clone());
            match value {
                Err(_) => continue,
                Ok(value) => {
                    for watcher in self.watchers.values() {
                        let (ref watched_id, ref range, ref cb) = *watcher;
                        if *watched_id != id {
                            continue;
                        }
                        if let Some(ref range) = *range {
                            let was_met = if let Some(Ok(ref value)) = old {
                                range.contains(value)
                            } else {
                                false
                            };
                            match (was_met, range.contains(&value)) {
                                (false, true) => {
                                    let _ = cb.send(WatchEvent::Enter {
                                        id: id.clone(),
                                        value: value.clone()
                                    });
                                }
                                (true, false) => {
                                    let _ = cb.send(WatchEvent::Exit {
                                        id: id.clone(),
                                        value: value.clone()
                                    });
                                }
                                _ => continue
                            }
                        } else {
                            let _ = cb.send(WatchEvent::Enter {
                                id: id.clone(),
                                value: value.clone()
                            });
                        }
                    }
                }
            }
        }
    }

    fn trigger_timers_until(&mut self, date: DateTime<UTC>) {
        self.trigger_timers_until = Some(date);
        loop {
            if let Some(ref timer) = self.timers.peek() {
                if timer.date > date {
                    break;
                }
            } else {
                break;
            }
            self.timers.pop().unwrap().trigger();
        }
    }

    fn execute(&mut self, op: AdapterOp) {
        use self::AdapterOp::*;
        match op {
            FetchValues { getters, tx } => {
                let _ = tx.send(self.fetch_values(getters, User::None));
            }
            SendValues { values, tx } => {
                let _ = tx.send(self.send_values(values, User::None));
            }
            Watch { source, tx } => {
                let _ = tx.send(self.register_watch(source));
            }
            Unwatch(key) => {
                let _ = self.remove_watch(key);
            }
            InjectGetterValues(values, tx) => {
                let _ = self.inject_getter_values(values);
                let _ = tx.send(FakeEnvEvent::Done);
            }
            InjectSetterErrors(errors, tx) => {
                let _ = self.inject_setter_errors(errors);
                let _ = tx.send(FakeEnvEvent::Done);
            }
            TriggerTimersUntil(date ,tx) => {
                let _ = self.trigger_timers_until(date.into());
                let _ = tx.send(FakeEnvEvent::Done);
            }
            ResetTimers(tx) => {
                self.trigger_timers_until = None;
                self.timers.clear();
                let _ = tx.send(FakeEnvEvent::Done);
            }
            AddTimer(timer) => {
                match self.trigger_timers_until {
                    None => {
                        self.timers.push(timer);
                    }
                    Some(ref date) if *date < timer.date => {
                        self.timers.push(timer);
                    }
                    Some(_) => {
                        timer.trigger()
                    }
                }
            }
        }
    }
}

/// Events of interest, that should be displayed to the user of the simulator.
#[derive(Debug)]
pub enum FakeEnvEvent {
    /// Some value was sent to a setter channel.
    Send {
        id: Id<Setter>,
        value: Value
    },

    /// Some error took place.
    Error(Error),

    /// Handling of an instruction is complete.
    Done,
}

/// An adapter for the simulator.
struct TestAdapter {
    id: Id<AdapterId>,

    /// The back-end holding the state of this adapter. Shared between all adapters.
    back_end: Arc<Mutex<Box<ExtSender<AdapterOp>>>>,
}
impl TestAdapter {
    fn new(id: Id<AdapterId>, back_end: Box<ExtSender<AdapterOp>>) -> Self {
        TestAdapter {
            id: id,
            back_end: Arc::new(Mutex::new(back_end)),
        }
    }
}

impl Adapter for TestAdapter {
    /// An id unique to this adapter. This id must persist between
    /// reboots/reconnections.
    fn id(&self) -> Id<AdapterId> {
        self.id.clone()
    }

    /// The name of the adapter.
    fn name(&self) -> &str {
        "Test Adapter"
    }
    fn vendor(&self) -> &str {
        "test@foxlink"
    }
    fn version(&self) -> &[u32;4] {
        &VERSION
    }
    // ... more metadata

    /// Request values from a group of channels.
    ///
    /// The AdapterManager always attempts to group calls to `fetch_values` by `Adapter`, and then
    /// expects the adapter to attempt to minimize the connections with the actual devices.
    ///
    /// The AdapterManager is in charge of keeping track of the age of values.
    fn fetch_values(&self, getters: Vec<Id<Getter>>, _: User) -> ResultMap<Id<Getter>, Option<Value>, Error> {
        let (tx, rx) = channel();
        self.back_end.lock().unwrap().send(AdapterOp::FetchValues {
            getters: getters,
            tx: Box::new(tx),
        }).unwrap();
        rx.recv().unwrap()
    }

    /// Request that values be sent to channels.
    ///
    /// The AdapterManager always attempts to group calls to `send_values` by `Adapter`, and then
    /// expects the adapter to attempt to minimize the connections with the actual devices.
    fn send_values(&self, values: HashMap<Id<Setter>, Value>, _: User) -> ResultMap<Id<Setter>, (), Error> {
        let (tx, rx) = channel();
        self.back_end.lock().unwrap().send(AdapterOp::SendValues {
            values: values,
            tx: Box::new(tx),
        }).unwrap();
        rx.recv().unwrap()
    }

    /// Watch a bunch of getters as they change.
    ///
    /// The `AdapterManager` always attempts to group calls to `fetch_values` by `Adapter`, and
    /// then expects the adapter to attempt to minimize the connections with the actual devices.
    /// The Adapter should however be ready to handle concurrent `register_watch` on the same
    /// devices, possibly with distinct `Option<Range>` options.
    ///
    /// If a `Range` option is set, the watcher expects to receive `EnterRange`/`ExitRange` events
    /// whenever the value available on the device enters/exits the range. If the `Range` is
    /// a `Range::Eq(x)`, the adapter may decide to reject the request or to interpret it as
    /// a `Range::BetweenEq { min: x, max: x }`.
    ///
    /// If no `Range` option is set, the watcher expects to receive `EnterRange` events whenever
    /// a new value is available on the device. The adapter may decide to reject the request if
    /// this is clearly not the expected usage for a device, or to throttle it.
    fn register_watch(&self, source: Vec<(Id<Getter>, Option<Range>, Box<ExtSender<WatchEvent>>)>) ->
            Vec<(Id<Getter>, Result<Box<AdapterWatchGuard>, Error>)>
    {
        let (tx, rx) = channel();
        self.back_end.lock().unwrap().send(AdapterOp::Watch {
            source: source,
            tx: Box::new(tx),
        }).unwrap();
        let received = rx.recv().unwrap();
        let tx_unregister = self.back_end.lock().unwrap().clone();
        received.iter().map(|&(ref id, ref result)| {
            let tx_unregister = tx_unregister.clone();
            (id.clone(), match *result {
                Err(ref err) => Err(err.clone()),
                Ok(ref key) => {
                    let guard = TestAdapterWatchGuard {
                        tx: Arc::new(Mutex::new(Box::new(tx_unregister))),
                        key: key.clone()
                    };
                    Ok(Box::new(guard) as Box<AdapterWatchGuard>)
                }
            })
        }).collect()
    }
}

/// A watchguard for a TestAdapter
#[derive(Clone)]
struct TestAdapterWatchGuard {
    tx: Arc<Mutex<Box<ExtSender<AdapterOp>>>>,
    key: usize,
}
impl AdapterWatchGuard for TestAdapterWatchGuard {}
impl Drop for TestAdapterWatchGuard {
    fn drop(&mut self) {
        let _ = self.tx.lock().unwrap().send(AdapterOp::Unwatch(self.key));
    }
}

#[derive(Clone)]
struct Timer {
    is_dropped: Arc<AtomicBool>,
    date: DateTime<UTC>,
    on_triggered: Box<ExtSender<()>>,
}
impl Timer {
    fn trigger(&self) {
        if self.is_dropped.load(AtomicOrdering::Relaxed) {
            return;
        }
        let _ = self.on_triggered.send(());
    }
}
impl Eq for Timer {}
impl Ord for Timer {
    fn cmp(&self, other: &Self) -> OrdOrdering {
        self.date.cmp(&other.date).reverse()
    }
}
impl PartialEq for Timer {
    fn eq(&self, other: &Self) -> bool {
        self.date == other.date
    }
}
impl PartialOrd for Timer {
    fn partial_cmp(&self, other: &Self) -> Option<OrdOrdering> {
        Some(self.cmp(other))
    }
}

pub struct TimerGuard(Arc<AtomicBool>);
impl Drop for TimerGuard {
    fn drop(&mut self) {
        self.0.store(true, AtomicOrdering::Relaxed)
    }
}

/// The test environment.
#[derive(Clone)]
pub struct FakeEnv {
    /// The manager in charge of all adapters.
    manager: Arc<AdapterManager>,

    ///
    on_event: Box<ExtSender<FakeEnvEvent>>,
    back_end: Box<ExtSender<AdapterOp>>,
}
impl fmt::Debug for FakeEnv {
    fn fmt(&self, _: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        Ok(())
    }
}
impl ExecutableDevEnv for FakeEnv {
    // Don't bother stopping watches.
    type WatchGuard = <AdapterManager as API>::WatchGuard;
    type API = AdapterManager;

    fn api(&self) -> &Self::API {
        &self.manager
    }

    type TimerGuard = TimerGuard;
    fn start_timer(&self, duration: Duration, timer: Box<ExtSender<()>>) -> Self::TimerGuard {
        let is_dropped = Arc::new(AtomicBool::new(false));
        let trigger = Timer {
            date: UTC::now() + duration.into(),
            on_triggered: timer,
            is_dropped: is_dropped.clone()
        };
        let _ = self.back_end.send(AdapterOp::AddTimer(trigger));
        TimerGuard(is_dropped)
    }
}
impl FakeEnv {
    pub fn new(on_event: Box<ExtSender<FakeEnvEvent>>) -> Self {
        let (tx, rx) = channel();
        let on_event_clone = on_event.clone();
        thread::spawn(move || {
            let mut back_end = TestSharedAdapterBackend::new(on_event_clone);
            for msg in rx {
                back_end.execute(msg);
            }
        });

        FakeEnv {
            on_event: on_event,
            manager: Arc::new(AdapterManager::new()),
            back_end: Box::new(tx),
        }
    }

    fn report_error<T>(&self, result: Result<T, Error>) {
        match result {
            Ok(_) => {},
            Err(err) => {
                let _ = self.on_event.send(FakeEnvEvent::Error(err));
            }
        }
    }

    /// Execute instructions issued by the user.
    pub fn execute(&self, instruction: Instruction) {
        use self::Instruction::*;
        match instruction {
            AddAdapters(vec) => {
                for id in vec {
                    let id = Id::new(&id);
                    let adapter = Arc::new(TestAdapter::new(id, self.back_end.clone()));
                    let result = self.manager.add_adapter(adapter);
                    self.report_error(result);
                }
                let _ = self.on_event.send(FakeEnvEvent::Done);
            },
            AddServices(vec) => {
                for service in vec {
                    let result = self.manager.add_service(service);
                    self.report_error(result);
                }
                let _ = self.on_event.send(FakeEnvEvent::Done);
            },
            AddGetters(vec) => {
                for getter in vec {
                    let result = self.manager.add_getter(getter);
                    self.report_error(result);
                }
                let _ = self.on_event.send(FakeEnvEvent::Done);
            },
            RemoveGetters(vec) => {
                for getter in vec {
                    let result = self.manager.remove_getter(&getter);
                    self.report_error(result);
                }
                let _ = self.on_event.send(FakeEnvEvent::Done);
            },
            AddSetters(vec) => {
                for setter in vec {
                    let result = self.manager.add_setter(setter);
                    self.report_error(result);
                }
                let _ = self.on_event.send(FakeEnvEvent::Done);
            },
            RemoveSetters(vec) => {
                for setter in vec {
                    let result = self.manager.remove_setter(&setter);
                    self.report_error(result);
                }
                let _ = self.on_event.send(FakeEnvEvent::Done);
            },
            InjectGetterValues(vec) => {
                self.back_end.send(AdapterOp::InjectGetterValues(vec, self.on_event.clone())).unwrap();
            },
            InjectSetterErrors(vec) => {
                self.back_end.send(AdapterOp::InjectSetterErrors(vec, self.on_event.clone())).unwrap();
            }
            TriggerTimersUntil(date) => {
                self.back_end.send(AdapterOp::TriggerTimersUntil(date, self.on_event.clone())).unwrap();
            }
            ResetTimers => {
                self.back_end.send(AdapterOp::ResetTimers(self.on_event.clone())).unwrap();
            }
//            _ => unimplemented!()
        }
    }
}
impl Serialize for FakeEnv {
    fn serialize<S>(&self, _: &mut S) -> Result<(), S::Error> where S: Serializer {
        panic!("Why are we attempting to serialize the env?")
    }
}
impl Deserialize for FakeEnv {
    fn deserialize<D>(_: &mut D) -> Result<Self, D::Error> where D: Deserializer {
         panic!("Why are we attempting to deserialize the env?")
    }
}

#[derive(Deserialize, Debug)]
/// Instructions given to the simulator by the user.
pub enum Instruction {
    AddAdapters(Vec<String>),
    AddServices(Vec<Service>),
    AddGetters(Vec<Channel<Getter>>),
    AddSetters(Vec<Channel<Setter>>),
    RemoveGetters(Vec<Id<Getter>>),
    RemoveSetters(Vec<Id<Setter>>),
    InjectGetterValues(Vec<(Id<Getter>, Result<Value, Error>)>),
    InjectSetterErrors(Vec<(Id<Setter>, Option<Error>)>),
    TriggerTimersUntil(TimeStamp),
    ResetTimers,
}

/// Operations internal to a TestAdapter.
enum AdapterOp {
    FetchValues {
        getters: Vec<Id<Getter>>,
        tx: Box<ExtSender<ResultMap<Id<Getter>, Option<Value>, Error>>>
    },
    SendValues {
        values: HashMap<Id<Setter>, Value>,
        tx: Box<ExtSender<ResultMap<Id<Setter>, (), Error>>>
    },
    Watch {
        source: Vec<(Id<Getter>, Option<Range>, Box<ExtSender<WatchEvent>>)>,
        tx: Box<ExtSender<Vec<(Id<Getter>, Result<usize, Error>)>>>
    },
    AddTimer(Timer),
    Unwatch(usize),
    InjectGetterValues(Vec<(Id<Getter>, Result<Value, Error>)>, Box<ExtSender<FakeEnvEvent>>),
    InjectSetterErrors(Vec<(Id<Setter>, Option<Error>)>, Box<ExtSender<FakeEnvEvent>>),
    TriggerTimersUntil(TimeStamp, Box<ExtSender<FakeEnvEvent>>),
    ResetTimers(Box<ExtSender<FakeEnvEvent>>),
}




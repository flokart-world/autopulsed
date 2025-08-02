use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;

use libpulse_binding::{
    context::{Context, introspect::{SinkInfo, SourceInfo}},
    callbacks::ListResult,
};
use log::{debug, error, info};

use crate::config::{Config, DeviceConfig};

struct AudioSink {
    name: String,
    recognized_as: Vec<String>,  // Config names
}

struct AudioSource {
    name: String,
    recognized_as: Vec<String>,  // Config names
}

fn check_device_match(device_config: &DeviceConfig, proplist: &libpulse_binding::proplist::Proplist) -> bool {
    if let Some(detect) = &device_config.detect {
        for (key, expected_value) in detect {
            if let Some(actual_value) = proplist.get_str(key) {
                if &actual_value != expected_value {
                    return false;
                }
            } else {
                return false;
            }
        }
        true
    } else {
        false
    }
}

pub struct State {
    context: Context,
    config: Config,
    found_sinks: HashMap<u32, AudioSink>,
    found_sources: HashMap<u32, AudioSource>,
    pending_default_sink_index: Option<u32>,
    pending_default_sink_callback: Option<Box<dyn FnMut(bool) + 'static>>,
    pending_default_source_index: Option<u32>,
    pending_default_source_callback: Option<Box<dyn FnMut(bool) + 'static>>,
}

impl State {
    fn new(context: Context, config: Config) -> Self {
        Self {
            context,
            config,
            found_sinks: HashMap::new(),
            found_sources: HashMap::new(),
            pending_default_sink_index: None,
            pending_default_sink_callback: None,
            pending_default_source_index: None,
            pending_default_source_callback: None,
        }
    }

    fn add_sink(&mut self, index: u32, sink_info: &SinkInfo) -> usize {
        let mut sink = AudioSink {
            name: sink_info.name.as_ref().map(|s| s.to_string()).unwrap_or_default(),
            recognized_as: Vec::new(),
        };

        info!("Found sink #{}, name = {}, description = {}", index, sink.name, sink_info.description.as_ref().map(|s| s.to_string()).unwrap_or_default());

        for (name, device_config) in &self.config.sinks {
            if check_device_match(device_config, &sink_info.proplist) {
                info!("Sink #{} is detected as '{}'", index, name);
                sink.recognized_as.push(name.clone());
            }
        }

        let match_count = sink.recognized_as.len();
        self.found_sinks.insert(index, sink);
        match_count
    }

    fn remove_sink(&mut self, index: u32) {
        if let Some(_) = self.found_sinks.remove(&index) {
            info!("Lost sink #{}", index);
        }
    }

    fn default_sink(&self) -> Option<(&String, u32)> {
        self.found_sinks.iter()
            .flat_map(|(&sink_index, sink)| {
                sink.recognized_as.iter()
                    .filter_map(move |config_name| {
                        self.config.sinks.get(config_name)
                            .and_then(|config| config.priority)
                            .map(|priority| (sink_index, config_name, priority))
                    })
            })
            .min_by_key(|&(_, _, priority)| priority)
            .map(|(index, config_name, _)| (config_name, index))
    }

    fn add_source(&mut self, index: u32, source_info: &SourceInfo) -> usize {
        let mut source = AudioSource {
            name: source_info.name.as_ref().map(|s| s.to_string()).unwrap_or_default(),
            recognized_as: Vec::new(),
        };

        info!("Found source #{}, name = {}, description = {}", index, source.name, source_info.description.as_ref().map(|s| s.to_string()).unwrap_or_default());

        for (name, device_config) in &self.config.sources {
            if check_device_match(device_config, &source_info.proplist) {
                info!("Source #{} is detected as '{}'", index, name);
                source.recognized_as.push(name.clone());
            }
        }

        let match_count = source.recognized_as.len();
        self.found_sources.insert(index, source);
        match_count
    }

    fn remove_source(&mut self, index: u32) {
        if let Some(_) = self.found_sources.remove(&index) {
            info!("Lost source #{}", index);
        }
    }

    fn default_source(&self) -> Option<(&String, u32)> {
        self.found_sources.iter()
            .flat_map(|(&source_index, source)| {
                source.recognized_as.iter()
                    .filter_map(move |config_name| {
                        self.config.sources.get(config_name)
                            .and_then(|config| config.priority)
                            .map(|priority| (source_index, config_name, priority))
                    })
            })
            .min_by_key(|&(_, _, priority)| priority)
            .map(|(index, config_name, _)| (config_name, index))
    }

    pub fn from_context(context: Context, config: Config) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self::new(context, config)))
    }
}

pub struct StateRunner<'scope> {
    origin: Rc<RefCell<State>>,
    state: &'scope mut State,
}

impl<'scope> StateRunner<'scope> {
    fn make_sink_callback(&self) -> impl FnMut(ListResult<&SinkInfo>) + 'static {
        let weak_origin = Rc::downgrade(&self.origin);
        let mut should_update = false;
        move |list_result| {
            if let Some(origin) = weak_origin.upgrade() {
                match list_result {
                    ListResult::Item(sink_info) => {
                        StateRunner::with(&origin, |runner| {
                            let match_count = runner.state.add_sink(sink_info.index, sink_info);
                            should_update = should_update || match_count > 0;
                        });
                    },
                    ListResult::End => {
                        debug!("Finished loading list result for sinks");
                        if should_update {
                            StateRunner::with(&origin, |runner| {
                                runner.update_default_sink();
                            });
                        }
                    },
                    ListResult::Error => {
                        error!("Error loading list result for sinks");
                    }
                }
            }
        }
    }

    fn query_all_sinks(&mut self) {
        let callback = self.make_sink_callback();
        let _op = self.state.context.introspect().get_sink_info_list(callback);
    }

    fn query_sink_by_index(&mut self, index: u32) {
        let callback = self.make_sink_callback();
        let _op = self.state.context.introspect().get_sink_info_by_index(index, callback);
    }

    fn make_source_callback(&self) -> impl FnMut(ListResult<&SourceInfo>) + 'static {
        let weak_origin = Rc::downgrade(&self.origin);
        let mut should_update = false;
        move |list_result| {
            if let Some(origin) = weak_origin.upgrade() {
                match list_result {
                    ListResult::Item(source_info) => {
                        StateRunner::with(&origin, |runner| {
                            let match_count = runner.state.add_source(source_info.index, source_info);
                            should_update = should_update || match_count > 0;
                        });
                    },
                    ListResult::End => {
                        debug!("Finished loading list result for sources");
                        if should_update {
                            StateRunner::with(&origin, |runner| {
                                runner.update_default_source();
                            });
                        }
                    },
                    ListResult::Error => {
                        error!("Error loading list result for sources");
                    }
                }
            }
        }
    }

    fn query_all_sources(&mut self) {
        let callback = self.make_source_callback();
        let _op = self.state.context.introspect().get_source_info_list(callback);
    }

    fn query_source_by_index(&mut self, index: u32) {
        let callback = self.make_source_callback();
        let _op = self.state.context.introspect().get_source_info_by_index(index, callback);
    }

    fn handle_set_default_sink_result(&mut self, success: bool, sink_index: u32) {
        if success {
            info!("Successfully set default sink to #{}", sink_index);
            if let Some(callback) = self.state.pending_default_sink_callback.take() {
                // Target changed during execution, retry with current target
                let new_sink_index = self.state.pending_default_sink_index.unwrap();
                if let Some(new_sink) = self.state.found_sinks.get(&new_sink_index) {
                    debug!("Setting default sink to #{}", new_sink_index);
                    let _op = self.state.context.set_default_sink(&new_sink.name, callback);
                }
            }
        } else {
            error!("Failed to set default sink");
            self.state.pending_default_sink_callback = None;
        }
    }

    fn update_default_sink(&mut self) {
        let default_sink = self.state.default_sink();

        if let Some((config_name, sink_index)) = default_sink {
            let weak_origin = Rc::downgrade(&self.origin);
            let callback = move |success: bool| {
                if let Some(origin) = weak_origin.upgrade() {
                    StateRunner::with(&origin, |runner| {
                        runner.handle_set_default_sink_result(success, sink_index);
                    });
                }
            };

            info!("Using sink '{}' as default", config_name);
            let pending = self.state.pending_default_sink_index.take().is_some();
            self.state.pending_default_sink_index = Some(sink_index);
            if pending {
                debug!("Default sink is being changed... deferring setting");
                self.state.pending_default_sink_callback = Some(Box::new(callback));
            } else if let Some(sink) = self.state.found_sinks.get(&sink_index) {
                debug!("Setting default sink to #{}", sink_index);
                let _op = self.state.context.set_default_sink(&sink.name, callback);
            }
        } else {
            self.state.pending_default_sink_index = None;
            self.state.pending_default_sink_callback = None;
        }
    }

    fn handle_set_default_source_result(&mut self, success: bool, source_index: u32) {
        if success {
            info!("Successfully set default source to #{}", source_index);
            if let Some(callback) = self.state.pending_default_source_callback.take() {
                // Target changed during execution, retry with current target
                let new_source_index = self.state.pending_default_source_index.unwrap();
                if let Some(new_source) = self.state.found_sources.get(&new_source_index) {
                    debug!("Setting default source to #{}", new_source_index);
                    let _op = self.state.context.set_default_source(&new_source.name, callback);
                }
            }
        } else {
            error!("Failed to set default source");
            self.state.pending_default_source_callback = None;
        }
    }

    fn update_default_source(&mut self) {
        let default_source = self.state.default_source();

        if let Some((config_name, source_index)) = default_source {
            let weak_origin = Rc::downgrade(&self.origin);
            let callback = move |success: bool| {
                if let Some(origin) = weak_origin.upgrade() {
                    StateRunner::with(&origin, |runner| {
                        runner.handle_set_default_source_result(success, source_index);
                    });
                }
            };

            info!("Using source '{}' as default", config_name);
            let pending = self.state.pending_default_source_index.take().is_some();
            self.state.pending_default_source_index = Some(source_index);
            if pending {
                debug!("Default source is being changed... deferring setting");
                self.state.pending_default_source_callback = Some(Box::new(callback));
            } else if let Some(source) = self.state.found_sources.get(&source_index) {
                debug!("Setting default source to #{}", source_index);
                let _op = self.state.context.set_default_source(&source.name, callback);
            }
        } else {
            self.state.pending_default_source_index = None;
            self.state.pending_default_source_callback = None;
        }
    }

    fn subscribe_to_events(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let interests =
            libpulse_binding::context::subscribe::InterestMaskSet::SINK
            | libpulse_binding::context::subscribe::InterestMaskSet::SOURCE;

        let weak_origin = Rc::downgrade(&self.origin);
        let _op = self.state.context.subscribe(interests, move |success| {
            if success {
                info!("Successfully subscribed to PulseAudio events");
                if let Some(origin) = weak_origin.upgrade() {
                    StateRunner::with(&origin, |runner| {
                        runner.query_all_sinks();
                        runner.query_all_sources();
                    });
                }
            } else {
                error!("Failed to subscribe to PulseAudio events");
            }
        });

        Ok(())
    }

    fn on_context_state_changed(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let context_state = self.state.context.get_state();

        debug!("The context state is <{:?}> now", context_state);

        if context_state == libpulse_binding::context::State::Ready {
            info!("Connected to PulseAudio server");
            self.subscribe_to_events()?;
        }

        Ok(())
    }

    pub fn connect(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let context = &mut self.state.context;

        // Since callbacks will be called within pa_context_connect(),
        // we call connect() before registering callbacks to prevent race
        // condition.
        context.connect(None, libpulse_binding::context::FlagSet::NOAUTOSPAWN, None)
            .map_err(|_| "Failed to connect to PulseAudio")?;

        let weak_origin = Rc::downgrade(&self.origin);
        context.set_state_callback(Some(Box::new(move || {
            if let Some(origin) = weak_origin.upgrade() {
                StateRunner::with(&origin, |runner| {
                    if let Err(e) = runner.on_context_state_changed() {
                        error!("Failed to handle state change: {}", e);
                    }
                });
            }
        })));

        let weak_origin = Rc::downgrade(&self.origin);
        context.set_subscribe_callback(Some(Box::new(move |facility, operation, index| {
            if let Some(origin) = weak_origin.upgrade() {
                StateRunner::with(&origin, |runner| {
                    match facility {
                        Some(libpulse_binding::context::subscribe::Facility::Sink) => {
                            match operation {
                                Some(libpulse_binding::context::subscribe::Operation::New) => {
                                    debug!("Got notified by new sink #{}", index);
                                    runner.query_sink_by_index(index);
                                },
                                Some(libpulse_binding::context::subscribe::Operation::Removed) => {
                                    debug!("Got notified by removed sink #{}", index);
                                    runner.state.remove_sink(index);
                                    runner.update_default_sink();
                                },
                                Some(libpulse_binding::context::subscribe::Operation::Changed) => {
                                    debug!("Got notified by changed sink #{}", index);
                                },
                                _ => {}
                            }
                        },
                        Some(libpulse_binding::context::subscribe::Facility::Source) => {
                            match operation {
                                Some(libpulse_binding::context::subscribe::Operation::New) => {
                                    debug!("Got notified by new source #{}", index);
                                    runner.query_source_by_index(index);
                                },
                                Some(libpulse_binding::context::subscribe::Operation::Removed) => {
                                    debug!("Got notified by removed source #{}", index);
                                    runner.state.remove_source(index);
                                    runner.update_default_source();
                                },
                                Some(libpulse_binding::context::subscribe::Operation::Changed) => {
                                    debug!("Got notified by changed source #{}", index);
                                },
                                _ => {}
                            }
                        },
                        _ => {}
                    }
                });
            }
        })));

        self.on_context_state_changed()?;
        Ok(())
    }

    pub fn with<Fn, Ret>(scope: &Rc<RefCell<State>>, proc: Fn) -> Ret
    where
        Fn: FnOnce(&mut StateRunner) -> Ret,
    {
        let mut runner = StateRunner {
            origin: Rc::clone(scope),
            state: &mut *scope.borrow_mut(),
        };
        proc(&mut runner)
    }
}

// autopulsed - A daemon for configuring PulseAudio automatically
// Copyright (C) 2025  Flokart World, Inc.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;

use libpulse_binding::{
    context::{Context, introspect::{SinkInfo, SourceInfo}},
    callbacks::ListResult,
};
use log::{debug, error, info};

use crate::config::{Config, DeviceConfig};

struct AudioDevice {
    original_name: String,
    recognized_as: Vec<String>, // Config names
}

struct AudioDeviceGroup {
    found_devices: HashMap<u32, AudioDevice>,
    pending_default_index: Option<u32>,
    pending_default_callback: Option<Box<dyn FnMut(bool) + 'static>>,
}

impl AudioDeviceGroup {
    fn new() -> Self {
        Self {
            found_devices: HashMap::new(),
            pending_default_index: None,
            pending_default_callback: None,
        }
    }
}

struct AudioDeviceRoot {
    sinks: AudioDeviceGroup,
    sources: AudioDeviceGroup,
}

impl AudioDeviceRoot {
    fn new() -> Self {
        Self {
            sinks: AudioDeviceGroup::new(),
            sources: AudioDeviceGroup::new(),
        }
    }
}

struct DeviceInfo<'a> {
    index: u32,
    name: Option<&'a str>,
    description: Option<&'a str>,
    proplist: &'a libpulse_binding::proplist::Proplist,
}

trait DeviceType {
    type Info<'a>;
    fn name_lower_case() -> &'static str;
    fn name_camel_case() -> &'static str;
    fn select(devices: &AudioDeviceRoot) -> &AudioDeviceGroup;
    fn select_mut(devices: &mut AudioDeviceRoot) -> &mut AudioDeviceGroup;
    fn get_definitions(config: &Config) -> &HashMap<String, DeviceConfig>;
    fn set_default(context: &mut Context, name: &String, callback: impl FnMut(bool) + 'static);
    fn extract_info<'a, 'b>(info: &'a Self::Info<'b>) -> DeviceInfo<'a>;
}

struct Sink;

impl DeviceType for Sink {
    type Info<'a> = SinkInfo<'a>;

    fn name_lower_case() -> &'static str {
        "sink"
    }

    fn name_camel_case() -> &'static str {
        "Sink"
    }

    fn select(devices: &AudioDeviceRoot) -> &AudioDeviceGroup {
        &devices.sinks
    }

    fn select_mut(devices: &mut AudioDeviceRoot) -> &mut AudioDeviceGroup {
        &mut devices.sinks
    }

    fn get_definitions(config: &Config) -> &HashMap<String, DeviceConfig> {
        &config.sinks
    }

    fn set_default(context: &mut Context, name: &String, callback: impl FnMut(bool) + 'static) {
        context.set_default_sink(name, callback);
    }

    fn extract_info<'a, 'b>(info: &'a Self::Info<'b>) -> DeviceInfo<'a> {
        DeviceInfo {
            index: info.index,
            name: info.name.as_deref(),
            description: info.description.as_deref(),
            proplist: &info.proplist,
        }
    }
}

struct Source;

impl DeviceType for Source {
    type Info<'a> = SourceInfo<'a>;

    fn name_lower_case() -> &'static str {
        "source"
    }

    fn name_camel_case() -> &'static str {
        "Source"
    }

    fn select(devices: &AudioDeviceRoot) -> &AudioDeviceGroup {
        &devices.sources
    }

    fn select_mut(devices: &mut AudioDeviceRoot) -> &mut AudioDeviceGroup {
        &mut devices.sources
    }

    fn get_definitions(config: &Config) -> &HashMap<String, DeviceConfig> {
        &config.sources
    }

    fn set_default(context: &mut Context, name: &String, callback: impl FnMut(bool) + 'static) {
        context.set_default_source(name, callback);
    }

    fn extract_info<'a, 'b>(info: &'a Self::Info<'b>) -> DeviceInfo<'a> {
        DeviceInfo {
            index: info.index,
            name: info.name.as_deref(),
            description: info.description.as_deref(),
            proplist: &info.proplist,
        }
    }
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
    all_devices: AudioDeviceRoot,
}

impl State {
    fn new(context: Context, config: Config) -> Self {
        Self {
            context,
            config,
            all_devices: AudioDeviceRoot::new(),
        }
    }

    fn add_device<'a, 'b, T>(&mut self, info: &'a T::Info<'b>) -> usize
    where
        T: DeviceType,
    {
        let device_info = T::extract_info(info);
        let devices = &mut T::select_mut(&mut self.all_devices).found_devices;
        let configs = T::get_definitions(&self.config);

        let mut device = AudioDevice {
            original_name: device_info.name.map(|s| s.to_string()).unwrap_or_default(),
            recognized_as: Vec::new(),
        };

        info!("Found {} #{}, name = {}, description = {}",
            T::name_lower_case(),
            device_info.index,
            device.original_name,
            device_info.description.unwrap_or_default()
        );

        for (name, device_config) in configs {
            if check_device_match(device_config, device_info.proplist) {
                info!("{} #{} is detected as '{}'",
                    T::name_camel_case(),
                    device_info.index,
                    name
                );
                device.recognized_as.push(name.clone());
            }
        }

        let match_count = device.recognized_as.len();
        devices.insert(device_info.index, device);
        match_count
    }

    fn remove_device<T>(&mut self, index: u32)
    where
        T: DeviceType,
    {
        let devices = &mut T::select_mut(&mut self.all_devices).found_devices;

        if let Some(_) = devices.remove(&index) {
            info!("Lost {} #{}", T::name_lower_case(), index);
        }
    }

    fn find_default_device<'a>(devices: &'a HashMap<u32, AudioDevice>, configs: &'a HashMap<String, DeviceConfig>) -> Option<(&'a String, u32)> {
        devices.iter()
            .flat_map(|(&device_index, device)| {
                device.recognized_as.iter()
                    .filter_map(move |config_name| {
                        configs.get(config_name)
                            .and_then(|config| config.priority)
                            .map(|priority| (device_index, config_name, priority))
                    })
            })
            .min_by_key(|&(_, _, priority)| priority)
            .map(|(index, config_name, _)| (config_name, index))
    }

    fn handle_set_default_result<T>(&mut self, device_index: u32, success: bool)
    where
        T: DeviceType,
    {
        let state = T::select_mut(&mut self.all_devices);
        if success {
            info!("Successfully set default {} to #{}", T::name_lower_case(), device_index);
            if let Some(callback) = state.pending_default_callback.take() {
                // Target changed during execution, retry with current target
                let new_device_index = state.pending_default_index.unwrap();
                if let Some(new_device) = state.found_devices.get(&new_device_index) {
                    debug!("Setting default {} to #{}", T::name_lower_case(), new_device_index);
                    let _op = T::set_default(&mut self.context, &new_device.original_name, callback);
                }
            }
        } else {
            error!("Failed to set default {}", T::name_lower_case());
            state.pending_default_callback = None;
        }
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
    fn update_default_device<T: DeviceType>(&mut self) {
        let State { all_devices: devices, context, .. } = self.state;
        let scope = T::select_mut(devices);
        let default_device = State::find_default_device(&scope.found_devices, T::get_definitions(&self.state.config));

        if let Some((config_name, device_index)) = default_device {
            let weak_origin = Rc::downgrade(&self.origin);
            let callback = move |success: bool| {
                if let Some(origin) = weak_origin.upgrade() {
                    StateRunner::with(&origin, |runner| {
                        runner.state.handle_set_default_result::<T>(device_index, success);
                    });
                }
            };

            info!("Using {} '{}' as default", T::name_lower_case(), config_name);
            let pending = scope.pending_default_index.take().is_some();
            scope.pending_default_index = Some(device_index);
            if pending {
                debug!("Default {} is being changed... deferring setting", T::name_lower_case());
                scope.pending_default_callback = Some(Box::new(callback));
            } else if let Some(device) = scope.found_devices.get(&device_index) {
                debug!("Setting default {} to #{}", T::name_lower_case(), device_index);
                T::set_default(context, &device.original_name, callback);
            }
        } else {
            scope.pending_default_index = None;
            scope.pending_default_callback = None;
        }
    }

    fn make_device_callback<T: DeviceType>(&self) -> impl for<'a, 'b> FnMut(ListResult<&'a T::Info<'b>>) + 'static {
        let weak_origin = Rc::downgrade(&self.origin);
        let mut should_update = false;
        move |list_result| {
            if let Some(origin) = weak_origin.upgrade() {
                match list_result {
                    ListResult::Item(info) => {
                        StateRunner::with(&origin, |runner| {
                            let match_count = runner.state.add_device::<T>(info);
                            should_update = should_update || match_count > 0;
                        });
                    },
                    ListResult::End => {
                        debug!("Finished loading list result for {}s", T::name_lower_case());
                        if should_update {
                            StateRunner::with(&origin, |runner| {
                                runner.update_default_device::<T>();
                            });
                        }
                    },
                    ListResult::Error => {
                        error!("Error loading list result for {}s", T::name_lower_case());
                    }
                }
            }
        }
    }

    fn query_all_sinks(&mut self) {
        let callback = self.make_device_callback::<Sink>();
        let _op = self.state.context.introspect().get_sink_info_list(callback);
    }

    fn query_sink_by_index(&mut self, index: u32) {
        let callback = self.make_device_callback::<Sink>();
        let _op = self.state.context.introspect().get_sink_info_by_index(index, callback);
    }

    fn query_all_sources(&mut self) {
        let callback = self.make_device_callback::<Source>();
        let _op = self.state.context.introspect().get_source_info_list(callback);
    }

    fn query_source_by_index(&mut self, index: u32) {
        let callback = self.make_device_callback::<Source>();
        let _op = self.state.context.introspect().get_source_info_by_index(index, callback);
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
                                    runner.state.remove_device::<Sink>(index);
                                    runner.update_default_device::<Sink>();
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
                                    runner.state.remove_device::<Source>(index);
                                    runner.update_default_device::<Source>();
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

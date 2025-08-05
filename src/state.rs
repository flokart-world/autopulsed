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

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use libpulse_binding::{
    callbacks::ListResult,
    context::{
        Context,
        introspect::{SinkInfo, SourceInfo},
    },
};
use log::{debug, error, info};

use crate::config::{Config, DeviceConfig, DeviceMatchConfig};

struct AudioDevice {
    original_name: String,
    recognized_as: Vec<String>, // Config names
}

struct AudioDeviceGroup {
    found_devices: HashMap<u32, AudioDevice>,
    remap_module_indices: HashMap<String, u32>,
    pending_default_index: Option<u32>,
    pending_default_callback: Option<Box<dyn FnMut(bool) + 'static>>,
}

impl AudioDeviceGroup {
    fn new() -> Self {
        Self {
            found_devices: HashMap::new(),
            remap_module_indices: HashMap::new(),
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
    owner_module: Option<u32>,
}

trait DeviceType {
    type Info<'a>;
    fn name_lower_case() -> &'static str;
    fn name_camel_case() -> &'static str;
    fn module_name() -> &'static str;
    #[allow(dead_code)]
    fn select(devices: &AudioDeviceRoot) -> &AudioDeviceGroup;
    fn select_mut(devices: &mut AudioDeviceRoot) -> &mut AudioDeviceGroup;
    fn get_definitions(config: &Config) -> &HashMap<String, DeviceConfig>;
    fn set_default(
        context: &mut Context,
        name: &str,
        callback: impl FnMut(bool) + 'static,
    );
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

    fn module_name() -> &'static str {
        "module-remap-sink"
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

    fn set_default(
        context: &mut Context,
        name: &str,
        callback: impl FnMut(bool) + 'static,
    ) {
        context.set_default_sink(name, callback);
    }

    fn extract_info<'a, 'b>(info: &'a Self::Info<'b>) -> DeviceInfo<'a> {
        DeviceInfo {
            index: info.index,
            name: info.name.as_deref(),
            description: info.description.as_deref(),
            proplist: &info.proplist,
            owner_module: info.owner_module,
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

    fn module_name() -> &'static str {
        "module-remap-source"
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

    fn set_default(
        context: &mut Context,
        name: &str,
        callback: impl FnMut(bool) + 'static,
    ) {
        context.set_default_source(name, callback);
    }

    fn extract_info<'a, 'b>(info: &'a Self::Info<'b>) -> DeviceInfo<'a> {
        DeviceInfo {
            index: info.index,
            name: info.name.as_deref(),
            description: info.description.as_deref(),
            proplist: &info.proplist,
            owner_module: info.owner_module,
        }
    }
}

struct DeviceMatchContext<'a> {
    device_config: &'a DeviceConfig,
    proplist: &'a libpulse_binding::proplist::Proplist,
    owner_module: Option<u32>,
    remap_module_indices: &'a HashMap<String, u32>,
    config_name: &'a str,
}

fn check_device_match(context: &DeviceMatchContext<'_>) -> bool {
    match &context.device_config.match_config {
        DeviceMatchConfig::Detect(detect) => {
            for (key, expected_value) in detect {
                if let Some(actual_value) = context.proplist.get_str(key) {
                    if &actual_value != expected_value {
                        return false;
                    }
                } else {
                    return false;
                }
            }
            true
        }
        DeviceMatchConfig::Remap(_) => {
            // Check if this device is created by our remap module
            match (
                context.owner_module,
                context.remap_module_indices.get(context.config_name),
            ) {
                (Some(owner), Some(&module)) => owner == module,
                _ => false,
            }
        }
    }
}

pub struct State {
    context: Context,
    config: Config,
    all_devices: AudioDeviceRoot,
    shutting_down: bool,
    num_pending_unloads: u32,
}

impl State {
    fn new(context: Context, config: Config) -> Self {
        Self {
            context,
            config,
            all_devices: AudioDeviceRoot::new(),
            shutting_down: false,
            num_pending_unloads: 0,
        }
    }

    fn add_device<'a, 'b, T>(&mut self, info: &'a T::Info<'b>) -> usize
    where
        T: DeviceType,
    {
        let device_info = T::extract_info(info);
        let configs = T::get_definitions(&self.config);

        let AudioDeviceGroup {
            found_devices: devices,
            remap_module_indices,
            ..
        } = T::select_mut(&mut self.all_devices);

        let mut device = AudioDevice {
            original_name: device_info
                .name
                .map(|s| s.to_string())
                .unwrap_or_default(),
            recognized_as: Vec::new(),
        };

        info!(
            "Found {} #{}, name = {}, description = {}",
            T::name_lower_case(),
            device_info.index,
            device.original_name,
            device_info.description.unwrap_or_default()
        );

        for (name, device_config) in configs {
            let match_context = DeviceMatchContext {
                device_config,
                proplist: device_info.proplist,
                owner_module: device_info.owner_module,
                remap_module_indices,
                config_name: name,
            };

            if check_device_match(&match_context) {
                info!(
                    "{} #{} is recognized as '{}'",
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

        if devices.remove(&index).is_some() {
            info!("Lost {} #{}", T::name_lower_case(), index);
        }
    }

    fn find_default_device<'a>(
        devices: &'a HashMap<u32, AudioDevice>,
        configs: &'a HashMap<String, DeviceConfig>,
    ) -> Option<(&'a String, u32)> {
        devices
            .iter()
            .flat_map(|(&device_index, device)| {
                device.recognized_as.iter().filter_map(move |config_name| {
                    configs
                        .get(config_name)
                        .and_then(|config| config.priority)
                        .map(|priority| (device_index, config_name, priority))
                })
            })
            .min_by_key(|&(_, _, priority)| priority)
            .map(|(index, config_name, _)| (config_name, index))
    }

    fn handle_set_default_result<T>(
        &mut self,
        device_index: u32,
        success: bool,
    ) where
        T: DeviceType,
    {
        let state = T::select_mut(&mut self.all_devices);
        if success {
            info!(
                "Successfully set default {} to #{}",
                T::name_lower_case(),
                device_index
            );
            if let Some(callback) = state.pending_default_callback.take() {
                // Target changed during execution, retry with current target
                let new_device_index = state.pending_default_index.unwrap();
                if let Some(new_device) =
                    state.found_devices.get(&new_device_index)
                {
                    debug!(
                        "Setting default {} to #{}",
                        T::name_lower_case(),
                        new_device_index
                    );
                    T::set_default(
                        &mut self.context,
                        &new_device.original_name,
                        callback,
                    );
                }
            }
        } else {
            error!("Failed to set default {}", T::name_lower_case());
            state.pending_default_callback = None;
        }
    }

    pub fn from_context(
        context: Context,
        config: Config,
    ) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self::new(context, config)))
    }
}

pub struct StateRunner<'scope> {
    origin: Rc<RefCell<State>>,
    state: &'scope mut State,
}

struct RemapModuleParams<'a> {
    config_name: &'a str,
    remap_config: &'a crate::config::RemapConfig,
    master_index: u32,
}

impl<'scope> StateRunner<'scope> {
    fn update_default_device<T: DeviceType>(&mut self) {
        let State {
            all_devices: devices,
            context,
            ..
        } = self.state;
        let scope = T::select_mut(devices);
        let default_device = State::find_default_device(
            &scope.found_devices,
            T::get_definitions(&self.state.config),
        );

        if let Some((config_name, device_index)) = default_device {
            let weak_origin = Rc::downgrade(&self.origin);
            let callback = move |success: bool| {
                if let Some(origin) = weak_origin.upgrade() {
                    StateRunner::with(&origin, |runner| {
                        runner.state.handle_set_default_result::<T>(
                            device_index,
                            success,
                        );
                    });
                }
            };

            info!(
                "Using {} '{}' as default",
                T::name_lower_case(),
                config_name
            );
            let pending = scope.pending_default_index.take().is_some();
            scope.pending_default_index = Some(device_index);
            if pending {
                debug!(
                    "Default {} is being changed... deferring setting",
                    T::name_lower_case()
                );
                scope.pending_default_callback = Some(Box::new(callback));
            } else if let Some(device) = scope.found_devices.get(&device_index)
            {
                debug!(
                    "Setting default {} to #{}",
                    T::name_lower_case(),
                    device_index
                );
                T::set_default(context, &device.original_name, callback);
            }
        } else {
            scope.pending_default_index = None;
            scope.pending_default_callback = None;
        }
    }

    fn make_device_callback<T: DeviceType>(
        &self,
    ) -> impl for<'a, 'b> FnMut(ListResult<&'a T::Info<'b>>) + 'static {
        let weak_origin = Rc::downgrade(&self.origin);
        let mut should_update = false;
        move |list_result| {
            if let Some(origin) = weak_origin.upgrade() {
                match list_result {
                    ListResult::Item(info) => {
                        StateRunner::with(&origin, |runner| {
                            let match_count =
                                runner.state.add_device::<T>(info);
                            should_update = should_update || match_count > 0;
                        });
                    }
                    ListResult::End => {
                        debug!(
                            "Finished loading list result for {}s",
                            T::name_lower_case()
                        );
                        if should_update {
                            StateRunner::with(&origin, |runner| {
                                runner.update_default_device::<T>();
                                runner.check_and_load_remaps::<T>();
                            });
                        }
                    }
                    ListResult::Error => {
                        error!(
                            "Error loading list result for {}s",
                            T::name_lower_case()
                        );
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
        let _op = self
            .state
            .context
            .introspect()
            .get_sink_info_by_index(index, callback);
    }

    fn query_all_sources(&mut self) {
        let callback = self.make_device_callback::<Source>();
        let _op = self
            .state
            .context
            .introspect()
            .get_source_info_list(callback);
    }

    fn query_source_by_index(&mut self, index: u32) {
        let callback = self.make_device_callback::<Source>();
        let _op = self
            .state
            .context
            .introspect()
            .get_source_info_by_index(index, callback);
    }

    fn handle_device_removed<T: DeviceType>(&mut self, index: u32) {
        self.state.remove_device::<T>(index);
        self.update_default_device::<T>();
        self.check_and_unload_remaps::<T>();
    }

    fn subscribe_to_events(
        &mut self,
    ) -> Result<(), Box<dyn std::error::Error>> {
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

    fn on_context_state_changed(
        &mut self,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let context_state = self.state.context.get_state();

        debug!("The context state is <{context_state:?}> now");

        if context_state == libpulse_binding::context::State::Ready {
            info!("Connected to PulseAudio server");
            self.subscribe_to_events()?;
        }

        Ok(())
    }

    pub fn connect(
        &mut self,
        server: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let context = &mut self.state.context;

        // Since callbacks will be called within pa_context_connect(),
        // we call connect() before registering callbacks to prevent race
        // condition.
        context
            .connect(
                server,
                libpulse_binding::context::FlagSet::NOAUTOSPAWN,
                None,
            )
            .map_err(|e| {
                format!(
                    "Failed to connect to PulseAudio server{}: {}",
                    server.map_or(String::new(), |s| format!(" at '{s}'")),
                    e
                )
            })?;

        let weak_origin = Rc::downgrade(&self.origin);
        context.set_state_callback(Some(Box::new(move || {
            if let Some(origin) = weak_origin.upgrade() {
                StateRunner::with(&origin, |runner| {
                    if let Err(e) = runner.on_context_state_changed() {
                        error!("Failed to handle state change: {e}");
                    }
                });
            }
        })));

        let weak_origin = Rc::downgrade(&self.origin);
        context.set_subscribe_callback(Some(Box::new(move |facility, operation, index| {
            if let Some(origin) = weak_origin.upgrade() {
                StateRunner::with(&origin, |runner| match facility {
                    Some(libpulse_binding::context::subscribe::Facility::Sink) => match operation {
                        Some(libpulse_binding::context::subscribe::Operation::New) => {
                            debug!("Got notified by new sink #{index}");
                            runner.query_sink_by_index(index);
                        }
                        Some(libpulse_binding::context::subscribe::Operation::Removed) => {
                            debug!("Got notified by removed sink #{index}");
                            runner.handle_device_removed::<Sink>(index);
                        }
                        Some(libpulse_binding::context::subscribe::Operation::Changed) => {
                            debug!("Got notified by changed sink #{index}");
                        }
                        _ => {}
                    },
                    Some(libpulse_binding::context::subscribe::Facility::Source) => match operation
                    {
                        Some(libpulse_binding::context::subscribe::Operation::New) => {
                            debug!("Got notified by new source #{index}");
                            runner.query_source_by_index(index);
                        }
                        Some(libpulse_binding::context::subscribe::Operation::Removed) => {
                            debug!("Got notified by removed source #{index}");
                            runner.handle_device_removed::<Source>(index);
                        }
                        Some(libpulse_binding::context::subscribe::Operation::Changed) => {
                            debug!("Got notified by changed source #{index}");
                        }
                        _ => {}
                    },
                    _ => {}
                });
            }
        })));

        self.on_context_state_changed()?;
        Ok(())
    }

    fn has_device_with_config_name(
        devices: &AudioDeviceGroup,
        config_name: &str,
    ) -> bool {
        // TODO: O(N) search could be problematic in environments with many devices.
        // Consider adding reverse index: HashMap<String, Vec<u32>> for config_name -> device_indices
        devices.found_devices.iter().any(|(_, device)| {
            device.recognized_as.iter().any(|name| name == config_name)
        })
    }

    fn find_device_index_by_config_name(
        devices: &AudioDeviceGroup,
        config_name: &str,
    ) -> Option<u32> {
        // TODO: O(N) search could be problematic in environments with many devices.
        // Consider adding reverse index: HashMap<String, Vec<u32>> for config_name -> device_indices
        devices
            .found_devices
            .iter()
            .find(|(_, device)| {
                device.recognized_as.iter().any(|name| name == config_name)
            })
            .map(|(&index, _)| index)
    }

    fn build_remap_module_args<T: DeviceType>(
        remap_config: &crate::config::RemapConfig,
        master_index: u32,
    ) -> String {
        let mut args = Vec::new();
        args.push(format!("master={master_index}"));

        if let Some(device_name) = &remap_config.device_name {
            args.push(format!(
                "{}_name={}",
                T::name_lower_case(),
                device_name
            ));
        }

        if let Some(device_properties) = &remap_config.device_properties {
            // Convert HashMap to PulseAudio property string format
            let props = device_properties
                .iter()
                .map(|(k, v)| {
                    // Escape single quotes in values
                    let escaped_value = v.replace("'", "'\\''\\'");
                    format!("{k}='{escaped_value}'")
                })
                .collect::<Vec<_>>()
                .join(" ");

            args.push(format!(
                "{}_properties=\"{}\"",
                T::name_lower_case(),
                props
            ));
        }

        if let Some(format) = &remap_config.format {
            args.push(format!("format={format}"));
        }

        if let Some(rate) = remap_config.rate {
            args.push(format!("rate={rate}"));
        }

        if let Some(channels) = remap_config.channels {
            args.push(format!("channels={channels}"));
        }

        if let Some(channel_map) = &remap_config.channel_map {
            args.push(format!("channel_map={channel_map}"));
        }

        if let Some(master_channel_map) = &remap_config.master_channel_map {
            args.push(format!("master_channel_map={master_channel_map}"));
        }

        if let Some(resample_method) = &remap_config.resample_method {
            args.push(format!("resample_method={resample_method}"));
        }

        if let Some(remix) = remap_config.remix {
            args.push(format!("remix={}", if remix { "yes" } else { "no" }));
        }

        args.join(" ")
    }

    fn load_remap_module<T: DeviceType>(
        &mut self,
        params: RemapModuleParams<'_>,
    ) {
        let argument = Self::build_remap_module_args::<T>(
            params.remap_config,
            params.master_index,
        );
        let weak_origin = Rc::downgrade(&self.origin);
        let config_name_owned = params.config_name.to_string();

        info!(
            "Loading {} remap module for '{}' with master #{}",
            T::name_lower_case(),
            params.config_name,
            params.master_index
        );

        let _op = self.state.context.introspect().load_module(
            T::module_name(),
            &argument,
            move |module_index| {
                if let Some(origin) = weak_origin.upgrade() {
                    StateRunner::with(&origin, |runner| {
                        let devices =
                            T::select_mut(&mut runner.state.all_devices);
                        devices
                            .remap_module_indices
                            .insert(config_name_owned.clone(), module_index);
                        info!(
                            "Successfully loaded {} remap module #{} for '{}'",
                            T::name_lower_case(),
                            module_index,
                            config_name_owned
                        );
                    });
                }
            },
        );
    }

    fn unload_remap_module<T: DeviceType>(&mut self, config_name: &str) {
        let module_index = {
            let devices = T::select(&self.state.all_devices);
            devices.remap_module_indices.get(config_name).copied()
        };

        if let Some(index) = module_index {
            let weak_origin = Rc::downgrade(&self.origin);
            let config_name_owned = config_name.to_string();

            info!(
                "Unloading {} remap module #{} for '{}'",
                T::name_lower_case(),
                index,
                config_name
            );

            let _op = self.state.context.introspect().unload_module(
                index,
                move |success| {
                    if let Some(origin) = weak_origin.upgrade() {
                        StateRunner::with(&origin, |runner| {
                            if success {
                                let devices = T::select_mut(&mut runner.state.all_devices);
                                devices.remap_module_indices.remove(&config_name_owned);
                                info!(
                                    "Successfully unloaded {} remap module #{} for '{}'",
                                    T::name_lower_case(),
                                    index,
                                    config_name_owned
                                );
                            } else {
                                error!(
                                    "Failed to unload {} remap module #{} for '{}'",
                                    T::name_lower_case(),
                                    index,
                                    config_name_owned
                                );
                            }
                        });
                    }
                },
            );
        }
    }

    fn check_and_load_remaps<T: DeviceType>(&mut self) {
        // Skip remap loading if shutting down
        if self.state.shutting_down {
            debug!("Skipping remap loading during shutdown");
            return;
        }

        let configs = T::get_definitions(&self.state.config);
        let devices = T::select(&self.state.all_devices);

        // Find all remap configs that should be loaded
        let mut remaps_to_load = Vec::new();

        for (config_name, config) in configs {
            if let crate::config::DeviceMatchConfig::Remap(remap) =
                &config.match_config
            {
                // Check if the master device exists
                let master_exists =
                    Self::has_device_with_config_name(devices, &remap.master);

                if master_exists
                    && !devices.remap_module_indices.contains_key(config_name)
                {
                    // Find the master device index
                    if let Some(master_index) =
                        Self::find_device_index_by_config_name(
                            devices,
                            &remap.master,
                        )
                    {
                        remaps_to_load.push((
                            config_name.clone(),
                            remap.clone(),
                            master_index,
                        ));
                    }
                }
            }
        }

        // Load all pending remaps
        for (config_name, remap, master_index) in remaps_to_load {
            self.load_remap_module::<T>(RemapModuleParams {
                config_name: &config_name,
                remap_config: &remap,
                master_index,
            });
        }
    }

    fn check_and_unload_remaps<T: DeviceType>(&mut self) {
        let configs = T::get_definitions(&self.state.config);
        let devices = T::select(&self.state.all_devices);

        // Find all remap modules that should be unloaded
        let mut remaps_to_unload = Vec::new();

        for config_name in devices.remap_module_indices.keys() {
            let should_unload = if let Some(config) = configs.get(config_name)
            {
                if let crate::config::DeviceMatchConfig::Remap(remap) =
                    &config.match_config
                {
                    // Check if the master device still exists
                    !Self::has_device_with_config_name(devices, &remap.master)
                } else {
                    true // Config changed from remap to detect
                }
            } else {
                true // Config removed
            };

            if should_unload {
                remaps_to_unload.push(config_name.clone());
            }
        }

        // Unload all pending remaps
        for config_name in remaps_to_unload {
            self.unload_remap_module::<T>(&config_name);
        }
    }

    pub fn with<Fn, Ret>(scope: &Rc<RefCell<State>>, proc: Fn) -> Ret
    where
        Fn: FnOnce(&mut StateRunner<'_>) -> Ret,
    {
        let mut runner = StateRunner {
            origin: Rc::clone(scope),
            state: &mut scope.borrow_mut(),
        };
        proc(&mut runner)
    }
}

impl State {
    pub fn begin_shutdown(&mut self) {
        self.shutting_down = true;
    }

    pub fn has_pending_unloads(&self) -> bool {
        self.num_pending_unloads > 0
    }
}

impl StateRunner<'_> {
    fn cleanup_device_type_modules<T: DeviceType>(&mut self) -> usize {
        let modules: Vec<_> = T::select(&self.state.all_devices)
            .remap_module_indices
            .clone()
            .into_iter()
            .collect();

        let mut count = 0;
        for (config_name, module_index) in modules {
            info!(
                "Unloading {} remap module #{} for '{}'",
                T::name_lower_case(),
                module_index,
                config_name
            );
            count += 1;
            self.state.num_pending_unloads += 1;

            let weak_origin = Rc::downgrade(&self.origin);
            let config_name_owned = config_name.clone();
            let device_type_name = T::name_lower_case();
            let _op = self.state.context.introspect().unload_module(module_index, move |success| {
                if let Some(origin) = weak_origin.upgrade() {
                    StateRunner::with(&origin, |runner| {
                        if success {
                            debug!("Successfully unloaded {device_type_name} remap module for '{config_name_owned}'");
                        } else {
                            error!("Failed to unload {device_type_name} remap module for '{config_name_owned}'");
                        }
                        runner.state.num_pending_unloads -= 1;

                        // Log when all modules are unloaded
                        if runner.state.num_pending_unloads == 0 && runner.state.shutting_down {
                            debug!("All remap modules unloaded");
                        }
                    });
                }
            });
        }
        count
    }

    pub fn cleanup_remap_modules(&mut self) {
        info!("Cleaning up remap modules on shutdown");

        let sink_count = self.cleanup_device_type_modules::<Sink>();
        let source_count = self.cleanup_device_type_modules::<Source>();
        let module_count = sink_count + source_count;

        if module_count == 0 {
            info!("No remap modules to clean up");
        } else {
            info!("Waiting for {module_count} remap modules to unload");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RemapConfig;
    use std::collections::HashMap;

    fn create_test_proplist(
        properties: &[(&str, &str)],
    ) -> libpulse_binding::proplist::Proplist {
        let mut proplist =
            libpulse_binding::proplist::Proplist::new().unwrap();
        for (key, value) in properties {
            proplist.set_str(key, value).unwrap();
        }
        proplist
    }

    fn create_test_match_context<'a>(
        config: &'a DeviceConfig,
        proplist: &'a libpulse_binding::proplist::Proplist,
        owner_module: Option<u32>,
        remap_module_indices: &'a HashMap<String, u32>,
        config_name: &'a str,
    ) -> DeviceMatchContext<'a> {
        DeviceMatchContext {
            device_config: config,
            proplist,
            owner_module,
            remap_module_indices,
            config_name,
        }
    }

    #[test]
    fn test_check_device_match_with_matching_properties() {
        let proplist = create_test_proplist(&[
            ("device.api", "alsa"),
            ("device.bus", "usb"),
        ]);

        // Create matching config
        let mut detect = HashMap::new();
        detect.insert("device.api".to_string(), "alsa".to_string());
        detect.insert("device.bus".to_string(), "usb".to_string());

        let config = DeviceConfig {
            priority: Some(1),
            match_config: DeviceMatchConfig::Detect(detect),
        };

        let empty_map = HashMap::new();
        let context = create_test_match_context(
            &config, &proplist, None, &empty_map, "test",
        );
        assert!(check_device_match(&context));
    }

    #[test]
    fn test_check_device_match_with_non_matching_properties() {
        let proplist = create_test_proplist(&[
            ("device.api", "alsa"),
            ("device.bus", "pci"),
        ]);

        let mut detect = HashMap::new();
        detect.insert("device.api".to_string(), "alsa".to_string());
        detect.insert("device.bus".to_string(), "usb".to_string()); // Different value

        let config = DeviceConfig {
            priority: Some(1),
            match_config: DeviceMatchConfig::Detect(detect),
        };

        let empty_map = HashMap::new();
        let context = create_test_match_context(
            &config, &proplist, None, &empty_map, "test",
        );
        assert!(!check_device_match(&context));
    }

    #[test]
    fn test_check_device_match_with_missing_property() {
        let proplist = create_test_proplist(&[
            ("device.api", "alsa"),
            // device.bus is not set
        ]);

        let mut detect = HashMap::new();
        detect.insert("device.api".to_string(), "alsa".to_string());
        detect.insert("device.bus".to_string(), "usb".to_string());

        let config = DeviceConfig {
            priority: Some(1),
            match_config: DeviceMatchConfig::Detect(detect),
        };

        let empty_map = HashMap::new();
        let context = create_test_match_context(
            &config, &proplist, None, &empty_map, "test",
        );
        assert!(!check_device_match(&context));
    }

    #[test]
    fn test_check_device_match_with_empty_detect() {
        let proplist = create_test_proplist(&[]);

        let config = DeviceConfig {
            priority: Some(1),
            match_config: DeviceMatchConfig::Detect(HashMap::new()),
        };

        // Empty detect matches everything
        let empty_map = HashMap::new();
        let context = create_test_match_context(
            &config, &proplist, None, &empty_map, "test",
        );
        assert!(check_device_match(&context));
    }

    #[test]
    fn test_check_device_match_with_remap() {
        let proplist = create_test_proplist(&[]);

        let config = DeviceConfig {
            priority: Some(1),
            match_config: DeviceMatchConfig::Remap(
                crate::config::RemapConfig {
                    master: "test".to_string(),
                    device_name: None,
                    device_properties: None,
                    format: None,
                    rate: None,
                    channels: None,
                    channel_map: None,
                    master_channel_map: None,
                    resample_method: None,
                    remix: None,
                },
            ),
        };

        // Remap configs never match during detection without owner_module
        let empty_map = HashMap::new();
        let context = create_test_match_context(
            &config, &proplist, None, &empty_map, "test",
        );
        assert!(!check_device_match(&context));
    }

    #[test]
    fn test_check_device_match_with_remap_and_owner_module() {
        let config = DeviceConfig {
            priority: Some(1),
            match_config: DeviceMatchConfig::Remap(RemapConfig {
                master: "master_device".to_string(),
                device_name: Some("remap_device".to_string()),
                device_properties: None,
                format: None,
                rate: None,
                channels: None,
                channel_map: None,
                master_channel_map: None,
                resample_method: None,
                remix: None,
            }),
        };

        let proplist = create_test_proplist(&[]);
        let mut remap_module_indices = HashMap::new();
        remap_module_indices.insert("remap_config".to_string(), 42);

        // Test with matching owner module
        let context = create_test_match_context(
            &config,
            &proplist,
            Some(42),
            &remap_module_indices,
            "remap_config",
        );
        assert!(check_device_match(&context));

        // Test with non-matching owner module
        let context = create_test_match_context(
            &config,
            &proplist,
            Some(43),
            &remap_module_indices,
            "remap_config",
        );
        assert!(!check_device_match(&context));

        // Test with no owner module
        let context = create_test_match_context(
            &config,
            &proplist,
            None,
            &remap_module_indices,
            "remap_config",
        );
        assert!(!check_device_match(&context));
    }

    #[test]
    fn test_find_default_device_with_priorities() {
        let mut devices = HashMap::new();
        devices.insert(
            1,
            AudioDevice {
                original_name: "device1".to_string(),
                recognized_as: vec![
                    "high_priority".to_string(),
                    "low_priority".to_string(),
                ],
            },
        );
        devices.insert(
            2,
            AudioDevice {
                original_name: "device2".to_string(),
                recognized_as: vec!["medium_priority".to_string()],
            },
        );

        let mut configs = HashMap::new();
        configs.insert(
            "high_priority".to_string(),
            DeviceConfig {
                priority: Some(1),
                match_config: DeviceMatchConfig::Detect(HashMap::new()),
            },
        );
        configs.insert(
            "medium_priority".to_string(),
            DeviceConfig {
                priority: Some(5),
                match_config: DeviceMatchConfig::Detect(HashMap::new()),
            },
        );
        configs.insert(
            "low_priority".to_string(),
            DeviceConfig {
                priority: Some(10),
                match_config: DeviceMatchConfig::Detect(HashMap::new()),
            },
        );

        let result = State::find_default_device(&devices, &configs);

        assert!(result.is_some());
        let (config_name, device_index) = result.unwrap();
        assert_eq!(config_name, "high_priority");
        assert_eq!(device_index, 1);
    }

    #[test]
    fn test_find_default_device_with_no_priority() {
        let mut devices = HashMap::new();
        devices.insert(
            1,
            AudioDevice {
                original_name: "device1".to_string(),
                recognized_as: vec!["config1".to_string()],
            },
        );

        let mut configs = HashMap::new();
        configs.insert(
            "config1".to_string(),
            DeviceConfig {
                priority: None,
                match_config: DeviceMatchConfig::Detect(HashMap::new()),
            },
        );

        let result = State::find_default_device(&devices, &configs);

        assert!(result.is_none());
    }

    #[test]
    fn test_find_default_device_with_empty_devices() {
        let devices = HashMap::new();
        let configs = HashMap::new();

        let result = State::find_default_device(&devices, &configs);

        assert!(result.is_none());
    }
}

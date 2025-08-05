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

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use clap::Parser;
use libpulse_binding::{
    context::Context,
    mainloop::{
        signal::{Event as SignalEvent, MainloopSignals},
        standard::{IterateResult, Mainloop},
    },
    proplist::Proplist,
};
use log::{debug, error, info};

mod config;
mod state;

use config::Config;
use state::{State, StateRunner};

#[derive(Parser)]
#[command(name = env!("CARGO_PKG_NAME"))]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = env!("CARGO_PKG_DESCRIPTION"))]
struct Args {
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    #[arg(
        short,
        long,
        value_name = "SERVER",
        help = "PulseAudio server to connect to"
    )]
    server: Option<String>,

    #[arg(short, long)]
    verbose: bool,
}

struct App {
    mainloop: Rc<RefCell<Mainloop>>,
    // State must be kept alive for PulseAudio callbacks to work properly.
    // Even though not directly accessed in run(), dropping it would cause
    // callbacks to fail since they hold weak references to this state.
    state: Rc<RefCell<State>>,
    // Signal handlers for SIGINT (Ctrl+C) and SIGTERM
    _sigint_handler: Option<SignalEvent>,
    _sigterm_handler: Option<SignalEvent>,
    // Flag to indicate quit was requested
    quit_requested: Rc<Cell<bool>>,
}

impl App {
    fn new(
        config: Config,
        server: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut proplist = Proplist::new().unwrap();
        proplist
            .set_str(
                libpulse_binding::proplist::properties::APPLICATION_NAME,
                env!("CARGO_PKG_NAME"),
            )
            .map_err(|_| "Failed to set application name")?;

        let mainloop = Rc::new(RefCell::new(
            Mainloop::new().ok_or("Failed to create mainloop")?,
        ));

        let context = {
            let mainloop_ref = mainloop.borrow();
            Context::new_with_proplist(
                &*mainloop_ref,
                env!("CARGO_PKG_NAME"),
                &proplist,
            )
            .ok_or("Failed to create context")?
        };

        let state = State::from_context(context, config);

        // Log server connection target if specified
        if let Some(ref server_str) = server {
            info!("Connecting to PulseAudio server: {server_str}");
        } else {
            info!("Connecting to default PulseAudio server");
        }

        // Connect to PulseAudio server during initialization
        StateRunner::with(&state, |runner| runner.connect(server.as_deref()))?;

        Ok(App {
            mainloop,
            state,
            _sigint_handler: None,
            _sigterm_handler: None,
            quit_requested: Rc::new(Cell::new(false)),
        })
    }

    fn setup_signal_handler(
        &mut self,
    ) -> Result<(), Box<dyn std::error::Error>> {
        const SIGINT: i32 = 2;
        const SIGTERM: i32 = 15;

        let create_signal_handler = |sig: i32, sig_name: &'static str| {
            let quit_flag = self.quit_requested.clone();
            SignalEvent::new(sig, move |_sig| {
                info!("Received {sig_name}, shutting down gracefully...");
                quit_flag.set(true);
            })
        };

        let sigint_handler = create_signal_handler(SIGINT, "SIGINT");
        let sigterm_handler = create_signal_handler(SIGTERM, "SIGTERM");

        self._sigint_handler = Some(sigint_handler);
        self._sigterm_handler = Some(sigterm_handler);

        // Initialize AFTER creating signal handlers to prevent race condition
        self.mainloop.borrow_mut().init_signals()?;

        Ok(())
    }

    fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.setup_signal_handler()?;

        info!(
            "{} started - monitoring audio device changes",
            env!("CARGO_PKG_NAME")
        );

        loop {
            if self.quit_requested.get() {
                info!("Signal received, initiating shutdown");
                break;
            }

            match self.mainloop.borrow_mut().iterate(true) {
                IterateResult::Quit(_) => {
                    info!("Mainloop quit");
                    break;
                }
                IterateResult::Err(_) => {
                    error!("Mainloop error");
                    return Err("Mainloop error".into());
                }
                IterateResult::Success(_) => {}
            }
        }

        info!("Cleaning up resources");
        self.state.borrow_mut().begin_shutdown();
        StateRunner::with(&self.state, |runner| {
            runner.cleanup_remap_modules();
        });

        if !self.state.borrow().has_pending_unloads() {
            info!("No modules to clean up, exiting");
            return Ok(());
        }

        loop {
            match self.mainloop.borrow_mut().iterate(true) {
                IterateResult::Quit(_) => {
                    info!("Mainloop quit");
                    break;
                }
                IterateResult::Err(_) => {
                    error!("Error during cleanup");
                    break;
                }
                IterateResult::Success(_) => {
                    if !self.state.borrow().has_pending_unloads() {
                        info!("All modules unloaded, cleanup completed");
                        break;
                    }
                }
            }
        }

        // Skip signal cleanup to avoid crashes
        // The main() function will call std::process::exit(0) which will
        // bypass all destructors, preventing the double-free issue in
        // libpulse-binding's signal handling code.
        debug!(
            "Skipping signal cleanup - will exit via std::process::exit(0)"
        );

        Ok(())
    }
}

fn load_config(
    config_path: Option<PathBuf>,
) -> Result<Config, Box<dyn std::error::Error>> {
    if let Some(path) = config_path {
        let content = std::fs::read_to_string(&path)?;
        let config: Config = serde_yaml::from_str(&content)?;
        info!("Loaded config from: {}", path.display());

        // Validate configuration
        config.validate()?;

        Ok(config)
    } else {
        info!("Using default configuration");
        Ok(Config::default())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    env_logger::Builder::from_default_env()
        .filter_level(if args.verbose {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Info
        })
        .init();

    info!(
        "Starting {} v{}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    );

    let config = load_config(args.config)?;
    let mut app = App::new(config, args.server)?;

    app.run()?;

    // WORKAROUND: Due to libpulse-binding's signal handling design flaw,
    // we need to exit immediately to avoid double-free crashes during cleanup.
    // The library has a fundamental issue where signals_done() and SignalEvent::drop()
    // both call pa_signal_free(), causing assertion failures. This violates Rust's
    // safety principles - safe code should never cause such crashes.
    // Using std::process::exit(0) bypasses all destructors, preventing the issue.
    std::process::exit(0);
}

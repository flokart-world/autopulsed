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
use std::path::PathBuf;
use std::rc::Rc;

use clap::Parser;
use libpulse_binding::{
    context::Context,
    mainloop::standard::{IterateResult, Mainloop},
    proplist::Proplist,
};
use log::{error, info};

mod config;
mod state;

use config::Config;
use state::{State, StateRunner};

#[derive(Parser)]
#[command(name = env!("CARGO_PKG_NAME"))]
#[command(about = env!("CARGO_PKG_DESCRIPTION"))]
struct Args {
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    #[arg(short, long)]
    verbose: bool,
}

struct App {
    mainloop: Mainloop,
    state: Rc<RefCell<State>>,
}

impl App {
    fn new(config: Config) -> Result<Self, Box<dyn std::error::Error>> {
        let mut proplist = Proplist::new().unwrap();
        proplist
            .set_str(
                libpulse_binding::proplist::properties::APPLICATION_NAME,
                env!("CARGO_PKG_NAME"),
            )
            .map_err(|_| "Failed to set application name")?;

        let mainloop = Mainloop::new().ok_or("Failed to create mainloop")?;

        let context = Context::new_with_proplist(
            &mainloop,
            env!("CARGO_PKG_NAME"),
            &proplist,
        )
        .ok_or("Failed to create context")?;

        let state = State::from_context(context, config);

        Ok(App { mainloop, state })
    }

    fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        StateRunner::with(&self.state, |runner| runner.connect())?;

        info!(
            "{} started - monitoring audio device changes",
            env!("CARGO_PKG_NAME")
        );

        loop {
            match self.mainloop.iterate(true) {
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
    let mut app = App::new(config)?;

    app.run()?;

    Ok(())
}

/* Copyright 2026 Justin Madru (justin.jdm64@gmail.com)
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use gaius::config::Config;
use gaius::harness::Harness;
use gaius::models::{Models, RecentModelDef};
use gaius::tui::TuiApp;
use pico_args::Arguments;
use std::error::Error;
use std::path::PathBuf;

struct Args {
    prompt: Option<String>,
    prompt_file: Option<PathBuf>,
    session_id: Option<String>,
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut pargs = Arguments::from_env();

    if pargs.contains(["-h", "--help"]) {
        print_help();
        std::process::exit(0);
    }

    let prompt_mode = pargs.contains("--prompt");
    let prompt_file = pargs.opt_value_from_os_str("--prompt-file", |path| {
        Ok::<PathBuf, std::convert::Infallible>(PathBuf::from(path))
    })?;
    let session_id = pargs.opt_value_from_str("--session")?;

    if prompt_mode && prompt_file.is_some() {
        return Err("--prompt and --prompt-file cannot both be present".into());
    }

    let prompt = if prompt_mode {
        Some(pargs.free_from_str()?)
    } else {
        None
    };

    let remaining = pargs.finish();
    if !remaining.is_empty() {
        return Err(format!("Unexpected arguments: {:?}", remaining).into());
    }

    Ok(Args {
        prompt,
        prompt_file,
        session_id,
    })
}

fn print_help() {
    println!("gaius - LLM agent harness");
    println!();
    println!("USAGE:");
    println!("  gaius [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("  --prompt                Run one prompt from the unnamed argument and exit");
    println!("  --prompt-file <PATH>    Run one prompt read from a file and exit");
    println!("  --session <ID>          Load and continue a saved session");
    println!("  -h, --help              Show this help message");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let initial_prompt = match (args.prompt, args.prompt_file) {
        (Some(prompt), None) => Some(prompt),
        (None, Some(path)) => Some(std::fs::read_to_string(path)?),
        (None, None) => None,
        (Some(_), Some(_)) => unreachable!("parse_args rejects conflicting prompt modes"),
    };

    let mut config = Config::new();
    config.load().await?;

    let selected_models = config.configured_models();
    let Some(selected_model) = selected_models.first() else {
        eprintln!("Unable to find configured model");
        std::process::exit(1);
    };

    let cached_models = Models::list(&config).await.unwrap_or_default();
    if cached_models.is_empty() {
        eprintln!("Unable to load model cache");
        std::process::exit(1);
    }

    let r_model = vec![RecentModelDef {
        provider: selected_model.provider_name.clone(),
        id: selected_model.model_id.clone(),
    }];

    let resolved_models = RecentModelDef::from_cache(&r_model, &cached_models);
    let Some(first_model) = resolved_models.first() else {
        eprintln!("Unable to load model from cache");
        std::process::exit(1);
    };

    let Ok(client) = selected_model.create_client() else {
        eprintln!("Unable to create client");
        std::process::exit(1);
    };

    let mut harness = Harness::new(
        client,
        first_model.clone(),
        config.agents().default_agent().clone(),
        initial_prompt,
        args.session_id,
    )?;

    if harness.is_oneshot() {
        harness.run().await?;
        if !harness.history().messages.is_empty()
            && let Some(session_id) = harness.session_id()
        {
            println!("To continue pass --session {}", session_id);
        }
    } else {
        // restore the last used model instead of what' in config
        if let Some(model) = RecentModelDef::load(&cached_models).first()
            && let Ok(client) = model.create_client(&config)
        {
            harness.set_model(client, model.clone());
        }

        let snapshot = TuiApp::new(config).run(harness).await?;
        if snapshot.has_history
            && let Some(session_id) = snapshot.session_id
        {
            println!("To continue pass --session {}", session_id);
        }
    }

    Ok(())
}

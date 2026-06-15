/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use gaius::config::Config;
use gaius::harness::Harness;
use gaius::models::Models;
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

    let agent = config.agents().default_agent().clone();
    let mut harness = if initial_prompt.is_some() && args.session_id.is_none() {
        Harness::new_without_session(agent)?
    } else {
        Harness::new(agent, args.session_id)?
    };
    let end_session: Option<String>;

    if let Some(prompt) = initial_prompt {
        let first_model = Models::first_from_config(&config).await?;
        harness.set_model(first_model.clone()).await?;
        harness.run(Some(prompt)).await?;
        end_session = harness.session_id();
    } else {
        // restore the last used model instead of what's in config
        if let Some(recent_model) = Models::first_from_recent(&config).await {
            harness.set_model(recent_model).await?;
        }

        let snapshot = TuiApp::new(config).run(harness).await?;
        end_session = snapshot.session_id;
    }

    if let Some(session_id) = end_session {
        println!("To continue pass --session {}", session_id);
    }

    Ok(())
}

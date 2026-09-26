/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use gaius::auth_codex::CodexAuth;
use gaius::cli_prompt::CliPrompt;
use gaius::config::Config;
use gaius::harness::Harness;
use gaius::models::Models;
use gaius::tui::TuiApp;
use pico_args::Arguments;
use std::error::Error;
use std::path::PathBuf;

struct Args {
    cli_mode: bool,
    prompt: Option<String>,
    session_id: Option<String>,
    login: Option<String>,
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut pargs = Arguments::from_env();

    if pargs.contains(["-h", "--help"]) {
        print_help();
        std::process::exit(0);
    }

    if pargs.contains(["-V", "--version"]) {
        println!("gaius {}", env!("GIT_VERSION"));
        std::process::exit(0);
    }

    let cli_mode = pargs.contains("--cli");
    let prompt_mode = pargs.contains("--prompt");
    let prompt_file = pargs.opt_value_from_os_str("--prompt-file", |path| {
        Ok::<PathBuf, std::convert::Infallible>(PathBuf::from(path))
    })?;
    let session_id = pargs.opt_value_from_str("--session")?;
    let login = pargs.opt_value_from_str("--login")?;

    let specified_modes = [
        cli_mode,
        prompt_mode,
        prompt_file.is_some(),
        login.is_some(),
    ]
    .iter()
    .filter(|&&specified| specified)
    .count();
    if specified_modes > 1 {
        return Err("--cli, --prompt, --prompt-file and --login are mutually exclusive".into());
    }

    let prompt = if prompt_mode {
        Some(pargs.free_from_str()?)
    } else if let Some(path) = prompt_file {
        Some(std::fs::read_to_string(path)?)
    } else {
        None
    };

    let remaining = pargs.finish();
    if !remaining.is_empty() {
        return Err(format!("Unexpected arguments: {:?}", remaining).into());
    }

    Ok(Args {
        cli_mode,
        prompt,
        session_id,
        login,
    })
}

fn print_help() {
    println!("gaius - LLM agent harness");
    println!();
    println!("USAGE:");
    println!("  gaius [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("  --cli                   Enter simple interactive mode");
    println!("  --prompt \"<PROMPT>\"     Run one prompt from quoted argument and exit");
    println!("  --prompt-file <PATH>    Run one prompt read from file and exit");
    println!();
    println!("  --session <ID>          Load and continue a saved session");
    println!();
    println!("  --login <PROVIDER>      Log in to a provider (codex) and exit");
    println!();
    println!("  -V, --version           Print version information");
    println!("  -h, --help              Show this help message");
}

async fn login(provider: &str) -> Result<(), Box<dyn Error>> {
    if provider.eq_ignore_ascii_case("codex") {
        CodexAuth::get()?.login().await?;
        return Ok(());
    }

    Err(format!(
        "Unknown provider '{}' to log in to. Supported: codex",
        provider
    )
    .into())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;

    if let Some(provider) = args.login.as_deref() {
        return login(provider).await;
    }

    let mut config = Config::new();
    config.load().await?;

    let agent = config.agents().default_agent().clone();
    let mut harness = if args.prompt.is_some() && args.session_id.is_none() {
        Harness::new_without_session(agent)?
    } else {
        Harness::new(agent, args.session_id)?
    };

    let models_cache = Models::list(&config).await?;
    let snapshot = if args.cli_mode || args.prompt.is_some() {
        let first_model = Models::first_from_config(&config, &models_cache)?;
        harness.set_model(first_model.clone()).await?;

        CliPrompt::new(args.prompt, harness).run().await?
    } else {
        if let Some(recent_model) = Models::first_from_recent(&models_cache) {
            harness.set_model(recent_model).await?;
        } else if let Ok(config_model) = Models::first_from_config(&config, &models_cache) {
            harness.set_model(config_model).await?;
        }

        TuiApp::new(config).run(harness).await?
    };

    if snapshot.has_history
        && let Some(session_id) = snapshot.session_id
    {
        println!("To continue pass --session {}", session_id);
    }

    Ok(())
}

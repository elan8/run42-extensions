mod diagnostics;

// The same process allocator as the upstream rumoca binary (mimalloc).
#[global_allocator]
static GLOBAL: rumoca_allocator::ProcessAllocator = rumoca_allocator::ProcessAllocator;

use anyhow::{bail, ensure, Result};
use clap::{Args, Parser, Subcommand};
use rumoca_sim::{
    DiffsolMethod, SessionCommand, SessionEvent, SimOptions, SimSolverMode, SimulationSession,
};
use serde::Deserialize;
use serde_json::json;
use std::io::{self, BufRead, BufWriter, Write};
use std::path::PathBuf;

const RUMOCA: &str = "0.10.4";

#[derive(Parser)]
#[command(version = concat!(env!("CARGO_PKG_VERSION"), " with rumoca 0.10.4"))]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Compile {
        #[command(flatten)]
        model: Model,
        #[arg(long, value_parser = ["flat-json"])]
        emit: String,
        #[arg(short = 'o')]
        output: PathBuf,
    },
    Sim {
        #[command(flatten)]
        model: Model,
        #[arg(long)]
        serve_stdio: bool,
        #[arg(long = "input", value_parser = initial_input)]
        inputs: Vec<(String, f64)>,
        #[arg(long)]
        rtol: Option<f64>,
        #[arg(long)]
        atol: Option<f64>,
    },
}

#[derive(Args)]
struct Model {
    file: String,
    #[arg(long)]
    model: String,
    #[arg(long = "source-root")]
    roots: Vec<String>,
}

fn initial_input(argument: &str) -> Result<(String, f64)> {
    let Some((name, value)) = argument.split_once('=') else {
        bail!("expected an initial input in NAME=VALUE form");
    };
    let value = value.parse::<f64>()?;
    ensure!(
        !name.is_empty() && value.is_finite(),
        "invalid initial input {argument}"
    );
    Ok((name.to_owned(), value))
}

fn compiler(model: &Model) -> rumoca::Compiler {
    rumoca::Compiler::new()
        .model(&model.model)
        .source_roots(&model.roots)
}

fn solver_mode(solver: Option<&str>) -> Result<SimSolverMode> {
    let normalized = solver
        .unwrap_or("auto")
        .trim()
        .to_ascii_lowercase()
        .replace(['-', '_', ' '], "");
    match normalized.as_str() {
        "auto" => Ok(SimSolverMode::Auto),
        "bdf" => Ok(SimSolverMode::Bdf),
        "rklike" => Ok(SimSolverMode::RkLike),
        "velimplicit" => Ok(SimSolverMode::VelImplicit),
        _ => bail!(
            "unsupported experiment solver {}",
            solver.unwrap_or_default()
        ),
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Request {
    Combined(ExtensionCommand),
    Rumoca(SessionCommand),
}

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
enum ExtensionCommand {
    CommunicationStep {
        time: f64,
        inputs: Vec<(String, f64)>,
        outputs: Vec<String>,
    },
}

fn communication_step(
    session: &mut SimulationSession,
    time: f64,
    inputs: Vec<(String, f64)>,
    outputs: Vec<String>,
) -> SessionEvent {
    if !time.is_finite() || time < session.time() {
        return SessionEvent::Error {
            code: "EX012".into(),
            message: "communication target must be finite and at or after the current time".into(),
        };
    }
    for command in [
        SessionCommand::SetInputs { inputs },
        SessionCommand::AdvanceTo { time },
    ] {
        match session.apply(command) {
            SessionEvent::Ok { .. } => {}
            event => return event,
        }
    }
    if outputs.is_empty() {
        return SessionEvent::Values {
            time: session.time(),
            values: Vec::new(),
        };
    }
    session.apply(SessionCommand::Values { names: outputs })
}

fn emit(output: &mut impl Write, event: &impl serde::Serialize) -> Result<()> {
    serde_json::to_writer(&mut *output, event)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}

fn serve(mut session: SimulationSession) -> Result<()> {
    let mut output = BufWriter::new(io::stdout().lock());
    let capabilities = if std::env::var("RUN42_RUMOCA_SEPARATE_STEPS").as_deref() == Ok("1") {
        vec![]
    } else {
        vec!["communication-step"]
    };
    emit(
        &mut output,
        &json!({
            "event": "hello", "protocol_version": 1, "engine": session.execution_receipt(),
            "capabilities": capabilities, "rumoca": RUMOCA, "time": session.time()
        }),
    )?;
    for line in io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let (event, combined) = match serde_json::from_str::<Request>(&line) {
            Ok(Request::Combined(ExtensionCommand::CommunicationStep {
                time,
                inputs,
                outputs,
            })) => (
                communication_step(&mut session, time, inputs, outputs),
                true,
            ),
            Ok(Request::Rumoca(command)) => (session.apply(command), false),
            Err(error) => (
                SessionEvent::Error {
                    code: "EX011".into(),
                    message: error.to_string(),
                },
                false,
            ),
        };
        emit(&mut output, &event)?;
        // A combined request can fail after modifying its input frame.
        if matches!(event, SessionEvent::Closed { .. })
            || (combined && matches!(event, SessionEvent::Error { .. }))
        {
            break;
        }
    }
    Ok(())
}

/// Report a compilation failure as one protocol event on stdout and exit 2.
/// Run42 turns it into a typed failure with codes and source labels.
fn fail_compilation(failure: diagnostics::CompilationFailure) -> ! {
    let mut output = io::stdout().lock();
    let event = json!({"event": "compilation_failure", "failure": &failure});
    let _ = emit(&mut output, &event);
    eprintln!("{}", failure.summary);
    std::process::exit(2)
}

/// Rumoca's flat-only facade reduces phase diagnostics to a string. On
/// failure only, ask the strict path for the original codes and source map.
/// This never accepts or simulates the failed model.
fn flat_failure(
    compiler: &rumoca::Compiler,
    file: &str,
    error: rumoca::CompilerError,
) -> diagnostics::CompilationFailure {
    if matches!(error, rumoca::CompilerError::FlattenError(_)) {
        if let Err(detail) = compiler.compile_file_dae(file) {
            let mut failure = diagnostics::compiler_error(detail);
            failure.summary = format!("{error}\n{}", failure.summary);
            return failure;
        }
    }
    diagnostics::compiler_error(error)
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Compile { model, output, .. } => {
            let compiler = compiler(&model);
            let flat = match compiler.compile_file_flat(&model.file) {
                Ok(flat) => flat,
                Err(error) => fail_compilation(flat_failure(&compiler, &model.file, error)),
            };
            let mut output = BufWriter::new(std::fs::File::create(output)?);
            serde_json::to_writer(&mut output, &flat)?;
            output.flush()?;
            Ok(())
        }
        Command::Sim {
            model,
            serve_stdio,
            inputs,
            rtol,
            atol,
        } => {
            ensure!(serve_stdio, "expected --serve-stdio");
            // A failed compile is reported in place of `hello`.
            let compiled = compiler(&model)
                .compile_file_dae(&model.file)
                .unwrap_or_else(|error| fail_compilation(diagnostics::compiler_error(error)));
            let t_start = compiled
                .experiment_start_time
                .filter(|t| t.is_finite())
                .unwrap_or(0.0);
            let t_end = compiled
                .experiment_stop_time
                .filter(|t| t.is_finite() && *t > t_start)
                .unwrap_or(t_start + 1.0);
            let mut options = SimOptions {
                t_start,
                t_end,
                initial_inputs: inputs,
                solver_mode: solver_mode(compiled.experiment_solver.as_deref())?,
                diffsol_method: DiffsolMethod::Bdf,
                ..SimOptions::default()
            };
            if let Some(rtol) = rtol {
                options.rtol = rtol;
            }
            if let Some(atol) = atol {
                options.atol = atol;
            }
            serve(SimulationSession::new(compiled.dae.as_ref(), options)?)
        }
    }
}

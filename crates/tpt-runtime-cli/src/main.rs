//! `tpt` — the tpt-runtime CLI (SPEC §29).
//!
//! Every command is a client of the daemon's local API; the CLI embeds no
//! runtime logic (SPEC §30). `tpt daemon start` re-launches this executable
//! detached with a hidden flag that runs the daemon in-process.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde_json::Value;
use tpt_runtime_api::ApiClient;
use tpt_runtime_config::{DaemonConfig, DEFAULT_PIPE_NAME};

#[derive(Parser)]
#[command(
    name = "tpt",
    version,
    about = "tpt-runtime: a unified workload runtime"
)]
struct Cli {
    /// Print raw JSON responses instead of formatted output.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Daemon lifecycle.
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// Daemon status summary.
    Status,
    /// Run a workload.
    Run(Box<RunArgs>),
    /// List workloads.
    List,
    /// Inspect a workload.
    Inspect { id: String },
    /// Print workload logs.
    Logs {
        id: String,
        /// Print stderr instead of stdout.
        #[arg(long)]
        stderr: bool,
        /// How many trailing lines to print.
        #[arg(long, default_value_t = 200)]
        tail: usize,
    },
    /// Stop a workload.
    Stop { id: String },
    /// Restart a workload.
    Restart { id: String },
    /// Destroy a finished workload's records.
    Destroy { id: String },
    /// Show runtime events.
    Events {
        /// Follow the live stream until interrupted.
        #[arg(long, short)]
        follow: bool,
        /// History limit when not following.
        #[arg(long, default_value_t = 30)]
        limit: usize,
    },
    /// Logical volumes (SPEC §15).
    Volume {
        #[command(subcommand)]
        action: VolumeAction,
    },
    /// Devices known to the runtime (SPEC §19).
    Devices,
    /// Secrets (SPEC §24).
    Secret {
        #[command(subcommand)]
        action: SecretAction,
    },
}

/// Arguments of `tpt run`.
#[derive(clap::Args)]
struct RunArgs {
    /// Run a native Windows executable.
    #[arg(long, value_name = "EXE")]
    windows: Option<String>,
    /// Run a WASM module (`.wasm` or WAT text).
    #[arg(long, value_name = "MODULE")]
    wasm: Option<String>,
    /// Run an OCI image (pulling requires tpt-boxcar; local store only).
    #[arg(long, value_name = "IMAGE")]
    oci: Option<String>,
    /// Run from a manifest file.
    #[arg(long, value_name = "FILE")]
    manifest: Option<String>,
    /// Workload name (defaults to a generated one).
    #[arg(long, short)]
    name: Option<String>,
    /// CPU cores.
    #[arg(long)]
    cpu: Option<f64>,
    /// Memory limit, e.g. `4GiB`.
    #[arg(long)]
    memory: Option<String>,
    /// Wall-clock timeout in seconds.
    #[arg(long)]
    timeout: Option<u64>,
    /// WASM fuel limit (instructions).
    #[arg(long)]
    fuel: Option<u64>,
    /// Network mode (none, host, private, outbound, service, isolated).
    #[arg(long)]
    network: Option<String>,
    /// Volume mount `name:/guest/path[:ro]` (repeatable).
    #[arg(long = "volume", value_name = "NAME:PATH[:RO]")]
    volumes: Vec<String>,
    /// Capability grant (repeatable), e.g. `network.outbound`.
    #[arg(long = "cap", value_name = "NAME")]
    capabilities: Vec<String>,
    /// Arguments after `--` for the workload program.
    #[arg(last = true)]
    args: Vec<String>,
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Start the daemon in the background.
    Start {
        /// State directory override.
        #[arg(long)]
        state_dir: Option<String>,
        /// Pipe name override.
        #[arg(long)]
        pipe: Option<String>,
    },
    /// Stop the daemon.
    Stop,
}

#[derive(Subcommand)]
enum VolumeAction {
    /// Create a volume.
    Create { name: String },
    /// List volumes.
    List,
    /// Remove an empty volume.
    Remove { name: String },
}

#[derive(Subcommand)]
enum SecretAction {
    /// Create or replace a secret; the value is read from stdin.
    Set { name: String },
    /// List secret names (never values).
    List,
    /// Delete a secret.
    Delete { name: String },
}

fn main() {
    let mut args: Vec<String> = std::env::args().collect();
    // hidden path: this binary re-invokes itself detached to host the daemon
    if args.len() == 2 && args[1] == "__tpt-daemon-worker" {
        if let Some(state_dir) = std::env::var_os("TPT_RUNTIME_DIR") {
            std::env::set_var("TPT_RUNTIME_DIR", state_dir);
        }
        let config = DaemonConfig::from_env();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        if let Err(err) = runtime.block_on(tpt_runtime_daemon::run(config)) {
            eprintln!("[daemon] fatal: {err}");
            std::process::exit(1);
        }
        return;
    }
    if args.len() == 2 && args[1] == "--tpt-pipe-check" {
        // used by `tpt daemon start` to locate its own executable
        println!("{}", DEFAULT_PIPE_NAME);
        return;
    }

    // clap sees normal args only
    let parsed = Cli::parse_from(args.clone());
    args.remove(0);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let code = match runtime.block_on(dispatch(parsed)) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("error: {err:#}");
            1
        }
    };
    std::process::exit(code);
}

async fn dispatch(cli: Cli) -> Result<()> {
    let config = DaemonConfig::from_env();
    match cli.command {
        Command::Daemon { action } => match action {
            DaemonAction::Start { state_dir, pipe } => start_daemon(state_dir, pipe),
            DaemonAction::Stop => {
                let mut client = ApiClient::connect(&config).await?;
                client.call("daemon.shutdown", Value::Null).await?;
                println!("daemon stopped");
                Ok(())
            }
        },
        Command::Status => {
            let mut client = ApiClient::connect(&config).await?;
            let status = client.call("daemon.status", Value::Null).await?;
            if cli.json {
                println!("{status:#}");
                return Ok(());
            }
            println!(
                "tpt-runtime v{} | uptime {}s | {} workload(s) | {}",
                status["version"].as_str().unwrap_or("?"),
                status["uptime_secs"].as_u64().unwrap_or(0),
                status["workloads"].as_u64().unwrap_or(0),
                config.pipe_name
            );
            Ok(())
        }
        Command::Run(run) => {
            let RunArgs {
                windows,
                wasm,
                oci,
                manifest,
                name,
                cpu,
                memory,
                timeout,
                fuel,
                network,
                volumes,
                capabilities,
                args,
            } = *run;
            let chosen = [&windows, &wasm, &oci, &manifest]
                .iter()
                .filter(|o| o.is_some())
                .count();
            if chosen != 1 {
                bail!("choose exactly one of --windows, --wasm, --oci, --manifest");
            }
            let manifest_toml = build_manifest(
                &windows,
                &wasm,
                &oci,
                &manifest,
                &name,
                cpu,
                &memory,
                timeout,
                fuel,
                &network,
                &volumes,
                &capabilities,
                args.clone(),
            )?;
            if std::env::var_os("TPT_DEBUG_MANIFEST").is_some() {
                eprintln!(
                    "--- manifest ---
{manifest_toml}--- end ---"
                );
            }
            let mut client = ApiClient::connect(&config).await?;
            let created = client
                .call(
                    "workloads.create",
                    serde_json::json!({ "manifest": manifest_toml }),
                )
                .await?;
            let id = created["id"].as_str().context("missing id").unwrap_or("?");
            let _ = client
                .call("workloads.start", serde_json::json!({ "id": id }))
                .await?;
            if !cli.json {
                println!("started {id}");
            } else {
                println!("{created}");
            }
            Ok(())
        }
        Command::List => {
            let mut client = ApiClient::connect(&config).await?;
            let list = client.call("workloads.list", Value::Null).await?;
            if cli.json {
                println!("{list:#}");
                return Ok(());
            }
            let items = list.as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                println!("no workloads");
                return Ok(());
            }
            println!(
                "{:<20} {:<14} {:<10} {:<8} PORTS",
                "ID", "NAME", "BACKEND", "STATE"
            );
            for item in items {
                println!(
                    "{:<20} {:<14} {:<10} {:<10} {}",
                    item["id"].as_str().unwrap_or("?"),
                    truncate(item["name"].as_str().unwrap_or("?"), 14),
                    item["backend"].as_str().unwrap_or("?"),
                    item["state"].as_str().unwrap_or("?"),
                    item["exposed_ports"]
                        .as_object()
                        .map(|ports| ports
                            .iter()
                            .map(|(k, v)| format!("{k}:{}", v.as_u64().unwrap_or(0)))
                            .collect::<Vec<_>>()
                            .join(","))
                        .unwrap_or_default(),
                );
            }
            Ok(())
        }
        Command::Inspect { id } => {
            let mut client = ApiClient::connect(&config).await?;
            let info = client
                .call("workloads.inspect", serde_json::json!({ "id": id }))
                .await?;
            if cli.json {
                println!("{info:#}");
                return Ok(());
            }
            println!(
                "workload  {} ({})",
                info["name"].as_str().unwrap_or("?"),
                info["id"].as_str().unwrap_or("?")
            );
            println!("  state       {}", info["state"].as_str().unwrap_or("?"));
            println!("  backend     {}", info["backend"].as_str().unwrap_or("?"));
            if let Some(code) = info["exit_code"].as_i64() {
                println!("  exit code   {code}");
            }
            if let Some(ports) = info["exposed_ports"].as_object() {
                if !ports.is_empty() {
                    println!(
                        "  ports       {}",
                        ports
                            .iter()
                            .map(|(k, v)| format!("{k}:{}", v.as_u64().unwrap_or(0)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
            }
            if let Some(mounts) = info["mounts"].as_array() {
                for mount in mounts {
                    println!(
                        "  volume      {} → {} ({} @ {})",
                        mount["name"].as_str().unwrap_or("?"),
                        mount["mount"].as_str().unwrap_or("?"),
                        mount["mode"].as_str().unwrap_or("read-write"),
                        mount["host_path"].as_str().unwrap_or("?"),
                    );
                }
            }
            if let Some(caps) = info["capabilities"].as_array() {
                if !caps.is_empty() {
                    println!(
                        "  caps        {}",
                        caps.iter()
                            .filter_map(|c| c.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
            }
            if let Some(usage) = info["usage"].as_object() {
                if !usage.is_empty() {
                    let mem = usage["memory_peak_bytes"].as_u64().unwrap_or(0);
                    println!("  mem peak    {mem} bytes");
                }
            }
            if let Some(details) = info["details"].as_object() {
                if let Some(pid) = details.get("pid") {
                    println!("  pid         {pid}");
                }
            }
            Ok(())
        }
        Command::Logs { id, stderr, tail } => {
            let mut client = ApiClient::connect(&config).await?;
            let logs = client
                .call(
                    "workloads.logs",
                    serde_json::json!({
                        "id": id,
                        "stream": if stderr { "stderr" } else { "stdout" },
                        "tail": tail,
                    }),
                )
                .await?;
            for line in logs["lines"].as_array().cloned().unwrap_or_default() {
                println!("{}", line.as_str().unwrap_or(""));
            }
            Ok(())
        }
        Command::Stop { id } => {
            let mut client = ApiClient::connect(&config).await?;
            let stopped = client
                .call("workloads.stop", serde_json::json!({ "id": id }))
                .await?;
            println!(
                "stopped {} (exit: {})",
                id,
                stopped["exit_code"]
                    .as_i64()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "killed".to_owned())
            );
            Ok(())
        }
        Command::Restart { id } => {
            let mut client = ApiClient::connect(&config).await?;
            client
                .call("workloads.restart", serde_json::json!({ "id": id }))
                .await?;
            println!("restarted {id}");
            Ok(())
        }
        Command::Destroy { id } => {
            let mut client = ApiClient::connect(&config).await?;
            client
                .call("workloads.destroy", serde_json::json!({ "id": id }))
                .await?;
            println!("destroyed {id}");
            Ok(())
        }
        Command::Events { follow, limit } => {
            let mut client = ApiClient::connect(&config).await?;
            if follow {
                client.subscribe().await?;
                loop {
                    match client.recv_event().await {
                        Ok(event) => println!("{}", event.event),
                        Err(err) => {
                            eprintln!("event stream ended: {err}");
                            return Ok(());
                        }
                    }
                }
            } else {
                let history = client
                    .call("events.history", serde_json::json!({ "limit": limit }))
                    .await?;
                for event in history["events"].as_array().cloned().unwrap_or_default() {
                    println!(
                        "{} {} {}",
                        event["timestamp"].as_str().unwrap_or("?"),
                        event["event"].as_str().unwrap_or("?"),
                        event["workload"].as_str().unwrap_or(""),
                    );
                }
                Ok(())
            }
        }
        Command::Volume { action } => {
            let mut client = ApiClient::connect(&config).await?;
            match action {
                VolumeAction::Create { name } => {
                    client
                        .call("volumes.create", serde_json::json!({ "name": name }))
                        .await?;
                    println!("created volume '{name}'");
                    Ok(())
                }
                VolumeAction::List => {
                    let volumes = client.call("volumes.list", Value::Null).await?;
                    if cli.json {
                        println!("{volumes:#}");
                        return Ok(());
                    }
                    for volume in volumes.as_array().cloned().unwrap_or_default() {
                        println!(
                            "{}  {}",
                            volume["name"].as_str().unwrap_or("?"),
                            volume["backing_path"].as_str().unwrap_or("?"),
                        );
                    }
                    Ok(())
                }
                VolumeAction::Remove { name } => {
                    client
                        .call("volumes.remove", serde_json::json!({ "name": name }))
                        .await?;
                    println!("removed volume '{name}'");
                    Ok(())
                }
            }
        }
        Command::Devices => {
            let mut client = ApiClient::connect(&config).await?;
            let devices = client.call("devices.list", Value::Null).await?;
            if cli.json {
                println!("{devices:#}");
                return Ok(());
            }
            for device in devices.as_array().cloned().unwrap_or_default() {
                println!(
                    "{}  [{}] {}",
                    device["id"].as_str().unwrap_or("?"),
                    device["class"].as_str().unwrap_or("?"),
                    device["description"].as_str().unwrap_or(""),
                );
            }
            Ok(())
        }
        Command::Secret { action } => {
            let mut client = ApiClient::connect(&config).await?;
            match action {
                SecretAction::Set { name } => {
                    println!("enter value for '{name}' (stdin):");
                    let mut value = String::new();
                    std::io::Read::read_to_string(&mut std::io::stdin(), &mut value)?;
                    let value = value.trim_end_matches(['\r', '\n']);
                    client
                        .call(
                            "secrets.set",
                            serde_json::json!({ "name": name, "value": value }),
                        )
                        .await?;
                    println!("secret '{name}' stored");
                    Ok(())
                }
                SecretAction::List => {
                    let secrets = client.call("secrets.list", Value::Null).await?;
                    for secret in secrets.as_array().cloned().unwrap_or_default() {
                        println!("{}", secret["name"].as_str().unwrap_or("?"));
                    }
                    Ok(())
                }
                SecretAction::Delete { name } => {
                    client
                        .call("secrets.delete", serde_json::json!({ "name": name }))
                        .await?;
                    println!("secret '{name}' deleted");
                    Ok(())
                }
            }
        }
    }
}

fn start_daemon(state_dir: Option<String>, pipe: Option<String>) -> Result<()> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    let exe = std::env::current_exe().context("cannot locate own executable")?;
    let mut command = Command::new(exe);
    command.arg("__tpt-daemon-worker");
    let mut env_config = DaemonConfig::from_env();
    if let Some(dir) = state_dir {
        env_config.state_dir = dir.into();
        command.env("TPT_RUNTIME_DIR", &env_config.state_dir);
    }
    if let Some(pipe) = pipe {
        env_config.pipe_name = pipe.clone();
        command.env("TPT_RUNTIME_PIPE", pipe);
    }

    #[cfg(windows)]
    command.creation_flags(0x0000_0008 | 0x0000_0200); // DETACHED_PROCESS | NEW_PROCESS_GROUP
    #[cfg(not(windows))]
    {
        use std::process::Stdio;
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
    }

    command.spawn().context("failed to spawn daemon")?;
    println!(
        "daemon starting on {} (state: {})",
        env_config.pipe_name,
        env_config.state_dir.display()
    );
    Ok(())
}

/// Builds a manifest from CLI flags so that `tpt run` shares the exact
/// manifest format with files (SPEC §32).
#[allow(clippy::too_many_arguments)]
fn build_manifest(
    windows: &Option<String>,
    wasm: &Option<String>,
    oci: &Option<String>,
    manifest: &Option<String>,
    name: &Option<String>,
    cpu: Option<f64>,
    memory: &Option<String>,
    timeout: Option<u64>,
    fuel: Option<u64>,
    network: &Option<String>,
    volumes: &[String],
    capabilities: &[String],
    args: Vec<String>,
) -> Result<String> {
    let mut toml_text = String::new();
    toml_text.push_str("api = \"tpt.runtime/v1\"\n\n");

    let default_name = format!(
        "wl-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    );
    toml_text.push_str(&format!(
        "[workload]\nname = \"{}\"\n\n",
        name.clone().unwrap_or(default_name)
    ));

    toml_text.push_str("[execution]\n");
    if let Some(exe) = windows {
        toml_text.push_str(&format!("backend = \"windows\"\nprogram = \"{exe}\"\n"));
        if !args.is_empty() {
            toml_text.push_str(&format!("args = {}\n", toml_array(&args)));
        }
    } else if let Some(module) = wasm {
        toml_text.push_str(&format!("backend = \"wasm\"\nmodule = \"{module}\"\n"));
        if !args.is_empty() {
            toml_text.push_str(&format!("args = {}\n", toml_array(&args)));
        }
    } else if let Some(image) = oci {
        toml_text.push_str(&format!("backend = \"oci\"\nimage = \"{image}\"\n"));
        if !args.is_empty() {
            toml_text.push_str(&format!("args = {}\n", toml_array(&args)));
        }
    } else if let Some(path) = manifest {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read manifest '{path}'"))?;
        let mut merged = text.trim_end().to_owned();
        // apply --name / resource / network / volume / cap overrides
        if let Some(name) = name {
            merged = override_key(&merged, "name", &format!("\"{name}\""));
        }
        toml_text = format!("api = \"tpt.runtime/v1\"\n\n{merged}\n");
        apply_overrides(
            &mut toml_text,
            &Overrides {
                cpu,
                memory: memory.as_deref(),
                timeout,
                fuel,
                network: network.as_deref(),
                volumes,
                capabilities,
            },
        )?;
        return Ok(toml_text);
    }
    toml_text.push('\n');

    apply_overrides(
        &mut toml_text,
        &Overrides {
            cpu,
            memory: memory.as_deref(),
            timeout,
            fuel,
            network: network.as_deref(),
            volumes,
            capabilities,
        },
    )?;
    Ok(toml_text)
}

struct Overrides<'a> {
    cpu: Option<f64>,
    memory: Option<&'a str>,
    timeout: Option<u64>,
    fuel: Option<u64>,
    network: Option<&'a str>,
    volumes: &'a [String],
    capabilities: &'a [String],
}

fn apply_overrides(toml_text: &mut String, overrides: &Overrides<'_>) -> Result<()> {
    let Overrides {
        cpu,
        memory,
        timeout,
        fuel,
        network,
        volumes,
        capabilities,
    } = overrides;
    if cpu.is_some() || memory.is_some() || timeout.is_some() || fuel.is_some() {
        toml_text.push_str("[resources]\n");
        if let Some(cpu) = cpu {
            toml_text.push_str(&format!("cpu = {cpu}\n"));
        }
        if let Some(memory) = memory {
            toml_text.push_str(&format!("memory = \"{memory}\"\n"));
        }
        if let Some(timeout) = timeout {
            toml_text.push_str(&format!("timeout_secs = {timeout}\n"));
        }
        if let Some(fuel) = fuel {
            toml_text.push_str(&format!("fuel = {fuel}\n"));
        }
        toml_text.push('\n');
    }
    if let Some(network) = network {
        toml_text.push_str(&format!("[network]\nmode = \"{network}\"\n\n"));
    }
    for volume in volumes.iter() {
        let (name, rest) = volume
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("--volume expects name:path[:ro], got '{volume}'"))?;
        let (mount, mode) = match rest.rsplit_once(':') {
            Some((mount, mode)) if mode == "ro" || mode == "read-only" => (mount, "read-only"),
            _ => (rest, "read-write"),
        };
        toml_text.push_str(&format!(
            "[[volumes]]\nname = \"{name}\"\nmount = \"{mount}\"\nmode = \"{mode}\"\n\n"
        ));
    }
    for cap in capabilities.iter() {
        toml_text.push_str(&format!("[[capabilities]]\nname = \"{cap}\"\n\n"));
    }
    Ok(())
}

fn override_key(text: &str, key: &str, value: &str) -> String {
    // crude but sufficient: replace `key = ...` first occurrence
    if let Some(pos) = text.find(&format!("{key} =")) {
        let line_end = text[pos..]
            .find('\n')
            .map(|e| pos + e)
            .unwrap_or(text.len());
        let mut merged = String::with_capacity(text.len());
        merged.push_str(&text[..pos]);
        merged.push_str(&format!("{key} = {value}"));
        merged.push_str(&text[line_end..]);
        merged
    } else {
        text.to_owned()
    }
}

fn toml_array(items: &[String]) -> String {
    let inner = items
        .iter()
        .map(|item| format!("\"{}\"", item.replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        text.to_owned()
    } else {
        format!("{}…", &text[..max.saturating_sub(1)])
    }
}

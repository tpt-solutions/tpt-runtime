//! Daemon binary entry point.

fn main() {
    let mut config = tpt_runtime_config::DaemonConfig::from_env();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--state-dir" => {
                if let Some(dir) = args.next() {
                    config.state_dir = dir.into();
                }
            }
            "--pipe" => {
                if let Some(pipe) = args.next() {
                    config.pipe_name = pipe;
                }
            }
            other => {
                eprintln!("unknown argument '{other}' (usage: tpt-runtime-daemon [--state-dir DIR] [--pipe NAME])");
                std::process::exit(2);
            }
        }
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    if let Err(err) = runtime.block_on(tpt_runtime_daemon::run(config)) {
        eprintln!("[daemon] fatal: {err}");
        std::process::exit(1);
    }
}

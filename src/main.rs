use clap::Parser;
use notify::{EventKind, RecursiveMode, Result, Watcher};
use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::thread;

#[derive(Parser, Debug)]
#[clap(author, version, about)]
struct Args {
    /// The verbosity level
    #[clap(short, long, default_value_t = 1)]
    verbose: i32,
    /// Path to the config file
    #[clap(short, long, default_value = "spyrun.toml")]
    config: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    dbg!(&args);
    let config_str = fs::read_to_string(args.config).unwrap();
    let config_toml: toml::Value = toml::from_str(&config_str).unwrap();
    dbg!(&config_toml);

    let (tx, rx) = mpsc::channel();

    let mut watcher = notify::recommended_watcher(tx)?;

    watcher.watch(Path::new("."), RecursiveMode::Recursive)?;

    thread::spawn(move || {
        for res in rx.into_iter() {
            match res {
                Ok(event) => match event.kind {
                    EventKind::Create(_) => {
                        println!("A file was created: {:?}", event.paths);
                    }
                    EventKind::Remove(_) => {
                        println!("A file was removed: {:?}", event.paths);
                    }
                    EventKind::Modify(_) => {
                        println!("A file was modified: {:?}", event.paths);
                    }
                    EventKind::Access(_) => {
                        println!("A file was accessed: {:?}", event.paths);
                    }
                    EventKind::Other => {
                        println!("Other event: {:?}", event);
                    }
                    EventKind::Any => {
                        println!("Unknown or unsupported event: {:?}", event);
                    }
                },
                Err(e) => println!("watch error: {:?}", e),
            }
        }
        println!("channel closed");
    });

    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();

    Ok(())
}

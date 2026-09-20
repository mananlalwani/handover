use std::fs;
use std::path::PathBuf;

use clap::{CommandFactory, ValueEnum};
use clap_complete::{Shell, generate_to};
use clap_mangen::Man;
use handoverctl::Cli;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os();
    let _program = args.next();
    let out_dir = PathBuf::from(args.next().ok_or("usage: handoverctl-gen <out-dir>")?);
    let man_dir = out_dir.join("man");
    let completion_dir = out_dir.join("completions");
    fs::create_dir_all(&man_dir)?;
    fs::create_dir_all(&completion_dir)?;

    let command = Cli::command();
    let man = Man::new(command.clone());
    let mut buffer = Vec::new();
    man.render(&mut buffer)?;
    fs::write(man_dir.join("handoverctl.1"), buffer)?;

    let daemon_man = Man::new(handoverctl::daemon::command())
        .title("HANDOVERD")
        .section("8");
    let mut daemon_buffer = Vec::new();
    daemon_man.render(&mut daemon_buffer)?;
    fs::write(man_dir.join("handoverd.8"), daemon_buffer)?;

    for shell in Shell::value_variants() {
        generate_to(*shell, &mut command.clone(), "handoverctl", &completion_dir)?;
    }

    println!("wrote man page and completions to {}", out_dir.display());
    Ok(())
}

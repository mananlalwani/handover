use std::fs;
use std::io::Write;
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
    add_device_completion(&completion_dir)?;

    println!("wrote man page and completions to {}", out_dir.display());
    Ok(())
}

fn add_device_completion(directory: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::OpenOptions::new()
        .append(true)
        .open(directory.join("_handoverctl"))?
        .write_all(br#"

# Live device arguments are supplied by the daemon.
_handoverctl_live_devices() {
    local -a values
    values=( ${(f)"$(handoverctl devices 2>/dev/null | awk 'NR > 1 && NF { print $1 }')"} )
    _describe 'Handover devices' values
}
_handoverctl_generated_device_completion() {
    case "${words[CURRENT-1]}" in
        send-url|send-file|notify|clipboard|calls|cancel-share|ping|ring|lock|keep-awake|tethering|filesystem-list|call|sync)
            _handoverctl_live_devices
            return 0
            ;;
    esac
    _handoverctl_generated "$@"
}
functions[_handoverctl_generated]=$functions[_handoverctl]
unfunction _handoverctl
function _handoverctl { _handoverctl_generated_device_completion "$@" }
"#)?;
    fs::OpenOptions::new()
        .append(true)
        .open(directory.join("handoverctl.bash"))?
        .write_all(br#"

_handoverctl_live_devices() {
    COMPREPLY=( $(compgen -W "$(handoverctl devices 2>/dev/null | awk 'NR > 1 && NF { print $1 }')" -- "${cur}") )
}
_handoverctl_generated_device_completion() {
    local prev="${COMP_WORDS[COMP_CWORD-1]}"
    local cur="${COMP_WORDS[COMP_CWORD]}"
    case "${prev}" in
        send-url|send-file|notify|clipboard|calls|cancel-share|ping|ring|lock|keep-awake|tethering|filesystem-list|call|sync)
            _handoverctl_live_devices
            return 0
            ;;
    esac
    _handoverctl_generated "$@"
}
eval "$(declare -f _handoverctl | sed '1s/^_handoverctl /_handoverctl_generated /')"
_handoverctl() { _handoverctl_generated_device_completion "$@"; }
"#)?;
    Ok(())
}

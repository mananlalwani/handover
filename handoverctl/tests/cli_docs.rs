use assert_cmd::Command;
use clap::{CommandFactory, ValueEnum};
use clap_complete::{Shell, generate};
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

fn handoverctl() -> Command {
    Command::cargo_bin("handoverctl").expect("handoverctl binary builds")
}

#[test]
fn top_level_help_lists_commands_and_exit_semantics() {
    handoverctl().arg("--help").assert().success().stdout(
        contains("devices")
            .and(contains("messages"))
            .and(contains("Exit status")),
    );
}

#[test]
fn nested_help_covers_main_families() {
    for args in [
        vec!["native", "--help"],
        vec!["messages", "--help"],
        vec!["media", "--help"],
        vec!["contacts", "--help"],
        vec!["clipboard-history", "--help"],
        vec!["custom", "--help"],
    ] {
        handoverctl()
            .args(&args)
            .assert()
            .success()
            .stdout(contains("--help"));
    }
}

#[test]
fn help_states_acceptance_not_completion() {
    handoverctl()
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("acceptance, not completion"));
    handoverctl()
        .args(["messages", "send", "--help"])
        .assert()
        .success()
        .stdout(contains("accepted, not delivered"));
}

#[test]
fn login_help_forbids_argv_secrets() {
    handoverctl()
        .args(["messages", "login", "--help"])
        .assert()
        .success()
        .stdout(contains("never argv").and(contains("--from-file")));
    // No password, token, or bundle value travels through argv.
    let output = handoverctl()
        .args(["messages", "login", "--help"])
        .output()
        .expect("login help runs");
    let help = String::from_utf8(output.stdout).expect("help is UTF-8");
    assert!(!help.to_lowercase().contains("password"), "{help}");
    assert!(!help.to_lowercase().contains("token"), "{help}");
    assert!(!help.contains("BUNDLE]"), "{help}");
}

#[test]
fn call_help_documents_confirm_requirement() {
    handoverctl()
        .args(["native", "call", "--help"])
        .assert()
        .success()
        .stdout(contains("--confirm"));
}

fn workspace_root() -> std::path::PathBuf {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .expect("handoverctl has a workspace parent")
        .to_path_buf()
}

#[test]
fn generated_man_page_matches_checked_in_artifact() {
    let command = handoverctl::Cli::command();
    let man = clap_mangen::Man::new(command);
    let mut buffer = Vec::new();
    man.render(&mut buffer).expect("man page renders");
    let checked_in = std::fs::read(workspace_root().join("docs/man/handoverctl.1"))
        .expect("man page is checked in");
    assert_eq!(
        buffer, checked_in,
        "docs/man/handoverctl.1 drifted from clap definitions; regenerate with \
         `cargo run -p handoverctl --bin handoverctl-gen -- <out-dir>`"
    );
}

#[test]
fn generated_completions_match_checked_in_artifacts() {
    let cases = [
        (Shell::Bash, "handoverctl.bash"),
        (Shell::Zsh, "_handoverctl"),
        (Shell::Fish, "handoverctl.fish"),
        (Shell::PowerShell, "_handoverctl.ps1"),
        (Shell::Elvish, "handoverctl.elv"),
    ];
    for shell in Shell::value_variants() {
        let mut command = handoverctl::Cli::command();
        let mut buffer = Vec::new();
        generate(*shell, &mut command, "handoverctl", &mut buffer);
        let name = cases
            .iter()
            .find(|(candidate, _)| candidate == shell)
            .expect("every shell has a checked-in file")
            .1;
        let checked_in = std::fs::read(workspace_root().join("completions").join(name))
            .unwrap_or_else(|_| panic!("completions/{name} is checked in"));
        assert_eq!(
            buffer, checked_in,
            "completions/{name} drifted from clap definitions; regenerate with \
             `cargo run -p handoverctl --bin handoverctl-gen -- <out-dir>`"
        );
    }
}

#[test]
fn shell_variant_coverage_stays_documented() {
    // If clap adds a shell, the drift test above must cover it too.
    let names: Vec<String> = Shell::value_variants()
        .iter()
        .map(|shell| {
            shell
                .to_possible_value()
                .expect("shell has a name")
                .get_name()
                .to_owned()
        })
        .collect();
    for expected in ["bash", "elvish", "fish", "powershell", "zsh"] {
        assert!(
            names.iter().any(|name| name == expected),
            "missing {expected} in {names:?}"
        );
    }
}

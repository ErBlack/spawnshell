use std::process::ExitCode;

#[derive(Debug, PartialEq, Eq)]
enum Mode {
    Daemon { adopting: bool },
    Cli,
    NativeUi,
}

fn mode(mut args: impl Iterator<Item = String>) -> Mode {
    match args.next().as_deref() {
        Some("__daemon") => Mode::Daemon {
            adopting: args.next().as_deref() == Some("--adopt"),
        },
        Some(_) => Mode::Cli,
        None => Mode::NativeUi,
    }
}

pub fn run(
    version: &'static str,
    description: &str,
    native_ui: impl FnOnce() -> ExitCode,
) -> ExitCode {
    match mode(std::env::args().skip(1)) {
        Mode::Daemon { adopting } => daemon::run(adopting, version),
        Mode::Cli => cli::run(version, description),
        Mode::NativeUi => native_ui(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode_of(args: &[&str]) -> Mode {
        mode(args.iter().map(ToString::to_string))
    }

    #[test]
    fn the_first_argument_picks_the_mode() {
        assert_eq!(mode_of(&[]), Mode::NativeUi);
        assert_eq!(mode_of(&["list"]), Mode::Cli);
        assert_eq!(mode_of(&["__daemon"]), Mode::Daemon { adopting: false });
        assert_eq!(
            mode_of(&["__daemon", "--adopt"]),
            Mode::Daemon { adopting: true }
        );
    }
}

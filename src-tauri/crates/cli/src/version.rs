use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, Read, Write};
use std::time::Duration;

use protocol::{Command, Connection, Failure, Reply, Role};

mod logo;

const DAEMON_WAIT: Duration = Duration::from_secs(2);
const LOGO_WIDTH: usize = 19;
const GAP: &str = "      ";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

enum Daemon {
    Running {
        pid: Option<u32>,
        version: Option<String>,
    },
    Updating,
    Unresponsive,
    Stopped,
}

struct Build {
    universal: bool,
    arch: &'static str,
}

pub(super) fn print(
    version: &str,
    description: &str,
    terminal: bool,
    out: &mut impl Write,
) -> io::Result<()> {
    if !terminal {
        return writeln!(out, "spawnshell {version}");
    }
    let paint = std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty());
    write!(
        out,
        "{}",
        screen(version, description, &build(), &daemon(), paint)
    )
}

fn build() -> Build {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        arch => arch,
    };
    let mut magic = [0; 4];
    let universal = std::env::current_exe()
        .and_then(File::open)
        .and_then(|mut exe| exe.read_exact(&mut magic))
        .is_ok_and(|()| matches!(magic, [0xca, 0xfe, 0xba, 0xbe | 0xbf]));
    Build { universal, arch }
}

fn daemon() -> Daemon {
    let Ok(socket) = protocol::socket() else {
        return Daemon::Stopped;
    };
    let Ok(mut connection) = Connection::connect(&socket, DAEMON_WAIT) else {
        return Daemon::Stopped;
    };
    match connection.call(Command::Hello { role: Role::Cli }) {
        Ok(Ok(Reply::Hello { pid, version, .. })) => Daemon::Running { pid, version },
        Ok(Err(Failure::Updating)) => Daemon::Updating,
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Daemon::Updating,
        _ => Daemon::Unresponsive,
    }
}

fn style(paint: bool, code: &str, text: &str) -> String {
    if paint {
        format!("{code}{text}{RESET}")
    } else {
        text.to_string()
    }
}

fn field(paint: bool, label: &str, value: &str, details: &[String]) -> String {
    let mut field = style(paint, DIM, &format!("{label:<8}")) + value;
    if !details.is_empty() {
        field += &style(paint, DIM, &format!(" · {}", details.join(" · ")));
    }
    field
}

fn color(rgb: u32) -> String {
    format!(
        "\x1b[38;2;{};{};{}m",
        rgb >> 16,
        (rgb >> 8) & 0xff,
        rgb & 0xff
    )
}

fn screen(version: &str, description: &str, build: &Build, daemon: &Daemon, paint: bool) -> String {
    let build = if build.universal {
        field(paint, "build", "universal", &[build.arch.to_string()])
    } else {
        field(paint, "build", build.arch, &[])
    };
    let daemon = match daemon {
        Daemon::Running { pid, version } => {
            let details: Vec<String> = [pid.map(|pid| format!("pid {pid}")), version.clone()]
                .into_iter()
                .flatten()
                .collect();
            field(paint, "daemon", "running", &details)
        }
        Daemon::Updating => field(paint, "daemon", "updating", &[]),
        Daemon::Unresponsive => field(paint, "daemon", "not responding", &[]),
        Daemon::Stopped => field(paint, "daemon", "not running", &[]),
    };
    let title = format!(
        "{} {}",
        style(paint, BOLD, "spawnshell"),
        style(paint, &color(logo::ACCENT), version)
    );
    let info = [
        (6, title),
        (7, description.to_string()),
        (9, build),
        (10, daemon),
    ];

    let mut screen = String::from("\n");
    for (row, (shape, colors)) in logo::SHAPE.iter().zip(logo::COLORS).enumerate() {
        let mut colors = colors.iter();
        for ch in shape.chars() {
            let rgb = if ch == ' ' { None } else { colors.next() };
            match rgb {
                Some(&rgb) if paint => {
                    let _ = write!(screen, "{}{ch}", color(rgb));
                }
                _ => screen.push(ch),
            }
        }
        if paint {
            screen.push_str(RESET);
        }
        if let Some((_, text)) = info.iter().find(|(line, _)| *line == row) {
            let pad = LOGO_WIDTH - shape.chars().count();
            let _ = write!(screen, "{}{GAP}{text}", " ".repeat(pad));
        }
        screen.push('\n');
    }
    screen.push('\n');
    screen
}

#[cfg(test)]
mod tests {
    use super::{Build, Daemon, LOGO_WIDTH, logo, screen};

    fn plain(text: &str) -> String {
        let mut plain = String::new();
        let mut escape = false;
        for ch in text.chars() {
            match ch {
                '\x1b' => escape = true,
                'm' if escape => escape = false,
                _ if escape => {}
                ch => plain.push(ch),
            }
        }
        plain
    }

    #[test]
    fn every_logo_character_has_a_color() {
        for (shape, colors) in logo::SHAPE.iter().zip(logo::COLORS) {
            assert_eq!(shape.chars().filter(|&ch| ch != ' ').count(), colors.len());
            assert!(shape.chars().count() <= LOGO_WIDTH);
        }
    }

    #[test]
    fn the_screen_shows_the_version_beside_the_logo() {
        let build = Build {
            universal: true,
            arch: "arm64",
        };
        let daemon = Daemon::Running {
            pid: Some(4312),
            version: Some("1.2.2".into()),
        };

        let text = screen("1.2.3", "A terminal", &build, &daemon, false);
        let lines: Vec<&str> = text.lines().collect();

        assert_eq!(lines[7], "    ;+++*###-------      spawnshell 1.2.3");
        assert_eq!(lines[8], "   +++++***#***++=,      A terminal");
        assert_eq!(
            lines[10],
            "--;-----;;-+++=          build   universal · arm64"
        );
        assert_eq!(
            lines[11],
            "      :;;-+++,           daemon  running · pid 4312 · 1.2.2"
        );
    }

    #[test]
    fn colors_only_add_escapes() {
        let build = Build {
            universal: false,
            arch: "x86_64",
        };

        let painted = screen("1.2.3", "A terminal", &build, &Daemon::Stopped, true);
        let unpainted = screen("1.2.3", "A terminal", &build, &Daemon::Stopped, false);

        assert!(painted.contains('\x1b'));
        assert!(!unpainted.contains('\x1b'));
        assert_eq!(plain(&painted), unpainted);
    }

    #[test]
    fn a_thin_build_and_a_stopped_daemon_have_no_details() {
        let build = Build {
            universal: false,
            arch: "x86_64",
        };

        let text = screen("1.2.3", "A terminal", &build, &Daemon::Stopped, false);

        assert!(text.contains("build   x86_64\n"));
        assert!(text.contains("daemon  not running\n"));
    }
}

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::report;
use crate::paths;

enum Integration {
    Zsh { dir: PathBuf },
}

impl Integration {
    fn install(shell: &str) -> Result<Option<Self>> {
        if Path::new(shell)
            .file_name()
            .is_none_or(|name| name != "zsh")
        {
            return Ok(None);
        }
        let dir = paths::data_dir()?.join("shell-integration").join("zsh");
        fs::create_dir_all(&dir).with_context(|| format!("can't create {}", dir.display()))?;
        let file = dir.join(".zshenv");
        fs::write(&file, include_str!("../zsh/.zshenv"))
            .with_context(|| format!("can't write {}", file.display()))?;
        Ok(Some(Integration::Zsh { dir }))
    }

    fn env(&self) -> Vec<(&'static str, String)> {
        match self {
            Integration::Zsh { dir } => {
                let mut env = vec![("ZDOTDIR", dir.to_string_lossy().into_owned())];
                if let Ok(original) = std::env::var("ZDOTDIR") {
                    env.push(("SPAWNSHELL_ZSH_ZDOTDIR", original));
                }
                env
            }
        }
    }

    fn before_restore(&self) -> &'static str {
        match self {
            Integration::Zsh { .. } => "builtin print -n '\\e]133;C\\a'\n",
        }
    }

    fn relaunch(&self, shell: &str) -> String {
        match self {
            Integration::Zsh { dir } => format!(
                "exec env ZDOTDIR={} {} -l",
                quote(&dir.to_string_lossy()),
                quote(shell)
            ),
        }
    }
}

pub(super) struct Shell {
    pub(super) path: String,
    integration: Option<Integration>,
}

impl Shell {
    pub(super) fn detect() -> Self {
        let path = platform::shell::default_path();
        let integration = Integration::install(&path).unwrap_or_else(|error| {
            report(&error.context("shell integration is off"));
            None
        });
        Shell { path, integration }
    }

    pub(super) fn env(&self) -> Vec<(&'static str, String)> {
        self.integration
            .as_ref()
            .map_or_else(Vec::new, Integration::env)
    }

    pub(super) fn arguments(&self, command: &str, restore: Option<&str>) -> Vec<String> {
        let mut args = vec!["-l".to_string()];
        let command = (!command.trim().is_empty()).then(|| command.to_string());
        match (restore, command) {
            (Some(restore), command) => {
                let next = command.unwrap_or_else(|| match &self.integration {
                    Some(integration) => integration.relaunch(&self.path),
                    None => format!("exec {} -l", quote(&self.path)),
                });
                let start = self
                    .integration
                    .as_ref()
                    .map_or("", Integration::before_restore);
                args.extend([
                    "-i".into(),
                    "-c".into(),
                    format!("{start}{restore}\n{next}"),
                ]);
            }
            (None, Some(command)) => args.extend(["-c".into(), command]),
            (None, None) => {}
        }
        args
    }
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_integrated_shell_changes_the_environment() {
        let mut shell = Shell {
            path: "/bin/sh".into(),
            integration: None,
        };
        assert_eq!(shell.env(), []);

        shell.integration = Some(Integration::Zsh {
            dir: PathBuf::from("/s"),
        });
        assert_eq!(shell.env()[0], ("ZDOTDIR", "/s".to_string()));
    }

    #[test]
    fn restore_runs_before_the_next_command() {
        let mut shell = Shell {
            path: "/bin/zsh".into(),
            integration: None,
        };
        assert_eq!(shell.arguments("", None), ["-l"]);
        assert_eq!(
            shell.arguments("", Some("claude --resume 'x'")),
            ["-l", "-i", "-c", "claude --resume 'x'\nexec '/bin/zsh' -l"]
        );

        shell.integration = Some(Integration::Zsh {
            dir: PathBuf::from("/s"),
        });
        assert_eq!(
            shell.arguments("", Some("claude")),
            [
                "-l",
                "-i",
                "-c",
                "builtin print -n '\\e]133;C\\a'\nclaude\nexec env ZDOTDIR='/s' '/bin/zsh' -l"
            ]
        );

        shell.integration = None;
        assert_eq!(shell.arguments("htop", None), ["-l", "-c", "htop"]);
        assert_eq!(
            shell.arguments("htop", Some("echo hi")),
            ["-l", "-i", "-c", "echo hi\nhtop"]
        );
    }
}

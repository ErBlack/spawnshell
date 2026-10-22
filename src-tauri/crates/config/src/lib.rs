use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::ValueEnum;
use global_hotkey::hotkey::HotKey;
use serde::{Deserialize, Serialize};

const FONT_SIZES: RangeInclusive<u32> = 6..=40;
const HEIGHTS: RangeInclusive<u32> = 10..=100;

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub shortcut: String,
    pub font_family: String,
    pub font_size: u32,
    pub height: u32,
    pub command: String,
    pub notification_sound: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            shortcut: "Control+Backquote".to_string(),
            font_family: "ui-monospace".to_string(),
            font_size: 16,
            height: 50,
            command: String::new(),
            notification_sound: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
#[value(rename_all = "snake_case")]
pub enum Key {
    Shortcut,
    FontFamily,
    FontSize,
    Height,
    Command,
    NotificationSound,
}

impl Key {
    fn name(self) -> String {
        self.to_possible_value()
            .map(|value| value.get_name().to_owned())
            .unwrap_or_default()
    }

    fn value(self, text: &str) -> Result<toml_edit::Value> {
        match self {
            Key::Shortcut => {
                check_shortcut(text)?;
                Ok(text.into())
            }
            Key::FontSize => self.number(text, FONT_SIZES).map(Into::into),
            Key::Height => self.number(text, HEIGHTS).map(Into::into),
            Key::FontFamily | Key::Command => Ok(text.into()),
            Key::NotificationSound => text
                .parse::<bool>()
                .map(Into::into)
                .map_err(|_| anyhow!("{} must be true or false", self.name())),
        }
    }

    fn number(self, text: &str, range: RangeInclusive<u32>) -> Result<i64> {
        let number = text.parse().map_err(|_| self.out_of_range(&range))?;
        self.check_range(number, &range).map(i64::from)
    }

    fn check_range(self, number: u32, range: &RangeInclusive<u32>) -> Result<u32> {
        if range.contains(&number) {
            Ok(number)
        } else {
            Err(self.out_of_range(range))
        }
    }

    fn out_of_range(self, range: &RangeInclusive<u32>) -> anyhow::Error {
        anyhow!(
            "{} must be a number from {} to {}",
            self.name(),
            range.start(),
            range.end()
        )
    }
}

fn check_shortcut(text: &str) -> Result<()> {
    text.parse::<HotKey>()
        .map(drop)
        .map_err(|error| anyhow!("{text:?} is not a valid shortcut: {error}"))
}

impl Config {
    fn check(&self) -> Result<()> {
        check_shortcut(&self.shortcut)?;
        Key::FontSize.check_range(self.font_size, &FONT_SIZES)?;
        Key::Height.check_range(self.height, &HEIGHTS)?;
        Ok(())
    }

    #[must_use]
    pub fn get(&self, key: Key) -> String {
        match key {
            Key::Shortcut => self.shortcut.clone(),
            Key::FontFamily => self.font_family.clone(),
            Key::FontSize => self.font_size.to_string(),
            Key::Height => self.height.to_string(),
            Key::Command => self.command.clone(),
            Key::NotificationSound => self.notification_sound.to_string(),
        }
    }
}

pub fn path() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join(".config/spawnshell/config.toml"))
        .context("can't find the home folder")
}

pub fn parse(path: &Path, text: &str) -> Result<Config> {
    let parsed = || -> Result<Config> {
        let table: toml::Table = toml::from_str(text)?;
        if let Some(unknown) = table
            .keys()
            .find(|name| Key::from_str(name, false).is_err())
        {
            bail!("unknown setting {unknown:?}");
        }
        let config: Config = table.try_into()?;
        config.check()?;
        Ok(config)
    };
    parsed().with_context(|| format!("{} is not valid", path.display()))
}

pub fn render(config: &Config) -> Result<String> {
    Ok(toml::to_string_pretty(config)?)
}

pub fn edit(path: &Path, current: &str, key: Key, text: &str) -> Result<(String, Config)> {
    let name = key.name();
    let mut value = key.value(text)?;
    let mut document: toml_edit::DocumentMut = current
        .parse()
        .with_context(|| format!("{} is not valid", path.display()))?;
    match document.get_mut(&name) {
        Some(toml_edit::Item::Value(existing)) => {
            *value.decor_mut() = existing.decor().clone();
            *existing = value;
        }
        Some(_) => bail!(
            "{name} in {} is not a plain value, edit it by hand",
            path.display()
        ),
        None => {
            document.insert(&name, toml_edit::Item::Value(value));
        }
    }
    let text = document.to_string();
    let config = parse(path, &text)?;
    Ok((text, config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_with_wrong_settings_is_rejected() {
        let path = Path::new("config.toml");
        assert!(parse(path, "heigth = 70\n").is_err());
        assert!(parse(path, "font_size = 100\n").is_err());
        assert!(parse(path, "shortcut = \"Control+Nothing\"\n").is_err());
        assert_eq!(parse(path, "height = 70\n").unwrap().height, 70);
    }

    #[test]
    fn every_setting_has_a_key() {
        let table: toml::Table = toml::from_str(&render(&Config::default()).unwrap()).unwrap();
        let mut settings: Vec<&str> = table.keys().map(String::as_str).collect();
        let mut keys: Vec<String> = Key::value_variants().iter().map(|key| key.name()).collect();
        settings.sort_unstable();
        keys.sort_unstable();
        assert_eq!(settings, keys);
    }

    #[test]
    fn setting_a_value_keeps_the_rest_of_the_file() {
        let path = Path::new("config.toml");
        let mine = "# mine\nfont_size = 14 # small\nheight = 70\n";

        let (text, _) = edit(path, mine, Key::FontSize, "18").unwrap();
        let (text, config) = edit(path, &text, Key::Command, "htop").unwrap();

        assert_eq!(config.font_size, 18);
        assert_eq!(config.height, 70);
        assert_eq!(config.command, "htop");
        assert_eq!(
            text,
            "# mine\nfont_size = 18 # small\nheight = 70\ncommand = \"htop\"\n"
        );
    }

    #[test]
    fn invalid_values_are_refused() {
        let path = Path::new("config.toml");
        assert!(edit(path, "height = 70\n", Key::FontSize, "100").is_err());
        assert!(edit(path, "height = 70\n", Key::Shortcut, "Control+Nothing").is_err());
        assert!(edit(path, "height = 70\n", Key::NotificationSound, "yes").is_err());
    }

    #[test]
    fn notification_sound_switches_off_and_on() {
        let path = Path::new("config.toml");
        let (text, config) = edit(path, "", Key::NotificationSound, "false").unwrap();
        assert!(!config.notification_sound);
        assert_eq!(config.get(Key::NotificationSound), "false");
        let (_, config) = edit(path, &text, Key::NotificationSound, "true").unwrap();
        assert!(config.notification_sound);
    }

    #[test]
    fn setting_never_replaces_a_table() {
        let path = Path::new("config.toml");
        assert!(edit(path, "[command]\nmine = 1\n", Key::Command, "htop").is_err());
    }
}

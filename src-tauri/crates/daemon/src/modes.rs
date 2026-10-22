use serde::{Deserialize, Serialize};

use super::notifications::Notification;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;
const MAX_OSC: usize = 64 * 1024;
const SCREEN_SWITCHES: [u16; 3] = [47, 1047, 1049];
const SAVE_CURSOR: u16 = 1048;
const KEYPAD: u16 = 66;
const SHIFT_OUT: u8 = 0x0e;
const SHIFT_IN: u8 = 0x0f;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slot {
    Dec(u16),
    Ansi(u16),
    SoftReset,
    CursorStyle,
    Charset(u8),
    Shift,
    Title,
    Palette(u16),
    Palettes,
    Color(u16),
}

impl Slot {
    fn cleared_by_reset(self) -> bool {
        matches!(
            self,
            Slot::Dec(_)
                | Slot::Ansi(_)
                | Slot::SoftReset
                | Slot::CursorStyle
                | Slot::Charset(_)
                | Slot::Shift
        )
    }

    fn replaces(self, earlier: Slot) -> bool {
        earlier == self || (self == Slot::Palettes && matches!(earlier, Slot::Palette(_)))
    }
}

fn number(text: &[u8]) -> Option<u16> {
    std::str::from_utf8(text).ok()?.parse().ok()
}

fn osc(payload: &str) -> String {
    format!("\x1b]{payload}\x07")
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct ProgramTitle {
    text: String,
    owned: bool,
    running: bool,
    #[serde(skip)]
    changed: bool,
}

impl ProgramTitle {
    fn set(&mut self, text: &str) {
        self.changed |= self.text != text;
        self.text = text.to_string();
        self.owned = self.running;
    }

    fn mark(&mut self, mark: &str) {
        if mark.starts_with('C') {
            self.running = true;
        }
        if !mark.starts_with('D') {
            return;
        }
        self.running = false;
        if std::mem::take(&mut self.owned) {
            self.set("");
        }
    }
}

#[derive(Default)]
struct Log {
    changes: Vec<(Slot, String)>,
    rows: u16,
    alternate: bool,
    regions: [Option<(u16, u16)>; 2],
    program_title: ProgramTitle,
    notifications: Vec<Notification>,
}

impl Log {
    fn record(&mut self, slot: Slot, bytes: String) {
        self.changes.retain(|(earlier, _)| !slot.replaces(*earlier));
        self.changes.push((slot, bytes));
    }

    fn region(&mut self) -> &mut Option<(u16, u16)> {
        &mut self.regions[usize::from(self.alternate)]
    }

    fn switch_screen(&mut self, alternate: bool) {
        if !alternate {
            self.regions[1] = None;
        }
        self.alternate = alternate;
    }

    fn scroll_region(&mut self, params: &vte::Params) {
        let mut values = params
            .iter()
            .map(|param| param.first().copied().unwrap_or(0));
        let top = values.next().filter(|&top| top > 0).unwrap_or(1);
        let bottom = values
            .next()
            .filter(|&bottom| bottom > 0 && bottom <= self.rows)
            .unwrap_or(self.rows);
        if bottom > top {
            *self.region() = (top != 1 || bottom != self.rows).then_some((top, bottom));
        }
    }

    fn osc(&mut self, payload: &[u8]) {
        let mut parts = payload.split(|byte| *byte == b';');
        let Some(command) = parts.next().and_then(number) else {
            return;
        };
        let text = String::from_utf8_lossy(payload);
        let rest = text.split_once(';').map_or("", |(_, rest)| rest);
        match command {
            0 | 2 => {
                self.record(Slot::Title, osc(&text));
                self.program_title.set(rest);
            }
            9 => self.iterm_notification(rest),
            133 => self.program_title.mark(rest),
            777 => self.rxvt_notification(rest),
            4 => {
                let parts: Vec<&[u8]> = parts.collect();
                for pair in parts.chunks(2) {
                    if let [index, spec] = pair
                        && *spec != b"?"
                        && let Some(index) = number(index)
                    {
                        let spec = String::from_utf8_lossy(spec);
                        self.record(Slot::Palette(index), osc(&format!("4;{index};{spec}")));
                    }
                }
            }
            104 => {
                let indices: Vec<u16> = parts.filter_map(number).collect();
                if indices.is_empty() {
                    self.record(Slot::Palettes, osc("104"));
                }
                for index in indices {
                    self.record(Slot::Palette(index), osc(&format!("104;{index}")));
                }
            }
            10..=12 => {
                for (color, spec) in (command..=12).zip(parts) {
                    if spec != b"?" {
                        let spec = String::from_utf8_lossy(spec);
                        self.record(Slot::Color(color), osc(&format!("{color};{spec}")));
                    }
                }
            }
            110..=112 => self.record(Slot::Color(command - 100), osc(&command.to_string())),
            _ => {}
        }
    }

    fn iterm_notification(&mut self, message: &str) {
        let code = message.split(';').next().unwrap_or_default();
        if !code.is_empty() && code.bytes().all(|byte| byte.is_ascii_digit()) {
            return;
        }
        self.notifications.extend(Notification::new(None, message));
    }

    fn rxvt_notification(&mut self, payload: &str) {
        let mut fields = payload.splitn(3, ';');
        if fields.next() != Some("notify") {
            return;
        }
        let title = fields.next();
        let body = fields.next().unwrap_or_default();
        self.notifications.extend(Notification::new(title, body));
    }

    fn reset(&mut self) {
        self.changes.retain(|(slot, _)| !slot.cleared_by_reset());
        self.alternate = false;
        self.regions = [None; 2];
    }
}

impl vte::Perform for Log {
    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        ignore: bool,
        action: char,
    ) {
        if ignore {
            return;
        }
        let set = if action == 'h' { 'h' } else { 'l' };
        match (intermediates, action) {
            ([b'?'], 'h' | 'l') => {
                for param in params {
                    let [mode] = param else {
                        continue;
                    };
                    if SCREEN_SWITCHES.contains(mode) {
                        self.switch_screen(set == 'h');
                    } else if *mode != SAVE_CURSOR {
                        let bytes = if *mode == KEYPAD {
                            if set == 'h' { "\x1b=" } else { "\x1b>" }.to_string()
                        } else {
                            format!("\x1b[?{mode}{set}")
                        };
                        self.record(Slot::Dec(*mode), bytes);
                    }
                }
            }
            ([], 'h' | 'l') => {
                for param in params {
                    if let [mode] = param {
                        self.record(Slot::Ansi(*mode), format!("\x1b[{mode}{set}"));
                    }
                }
            }
            ([], 'r') => self.scroll_region(params),
            ([b'!'], 'p') => {
                self.record(Slot::SoftReset, "\x1b[!p".to_string());
                *self.region() = None;
            }
            ([b' '], 'q') => {
                let style = params.iter().next().and_then(|param| param.first());
                let style = style.copied().unwrap_or(0);
                self.record(Slot::CursorStyle, format!("\x1b[{style} q"));
            }
            _ => {}
        }
    }

    fn execute(&mut self, byte: u8) {
        if matches!(byte, SHIFT_OUT | SHIFT_IN) {
            self.record(Slot::Shift, char::from(byte).to_string());
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        if ignore {
            return;
        }
        match (intermediates, byte) {
            ([], b'=') => self.record(Slot::Dec(KEYPAD), "\x1b=".to_string()),
            ([], b'>') => self.record(Slot::Dec(KEYPAD), "\x1b>".to_string()),
            ([], b'c') => self.reset(),
            ([], b'n' | b'o' | b'|' | b'}' | b'~') => {
                self.record(Slot::Shift, format!("\x1b{}", char::from(byte)));
            }
            ([selector @ (b'(' | b')' | b'*' | b'+' | b'-' | b'.' | b'/')], _) => {
                let level = match selector {
                    b'(' => 0,
                    b')' | b'-' => 1,
                    b'*' | b'.' => 2,
                    _ => 3,
                };
                self.record(
                    Slot::Charset(level),
                    format!("\x1b{}{}", char::from(*selector), char::from(byte)),
                );
            }
            _ => {}
        }
    }
}

#[derive(Default)]
enum Scan {
    #[default]
    Ground,
    Escape,
    Osc,
}

#[derive(Default)]
struct OscScanner {
    scan: Scan,
    payload: Vec<u8>,
    overflow: bool,
}

impl OscScanner {
    fn finish(&mut self, log: &mut Log) {
        if !self.overflow {
            log.osc(&self.payload);
        }
    }

    fn process(&mut self, bytes: &[u8], log: &mut Log) {
        for &byte in bytes {
            self.scan = match self.scan {
                Scan::Ground if byte == ESC => Scan::Escape,
                Scan::Ground => Scan::Ground,
                Scan::Escape => match byte {
                    b']' => {
                        self.payload.clear();
                        self.overflow = false;
                        Scan::Osc
                    }
                    ESC => Scan::Escape,
                    _ => Scan::Ground,
                },
                Scan::Osc => match byte {
                    BEL => {
                        self.finish(log);
                        Scan::Ground
                    }
                    ESC => {
                        self.finish(log);
                        Scan::Escape
                    }
                    CAN | SUB => Scan::Ground,
                    0..0x20 => Scan::Osc,
                    _ => {
                        if self.payload.len() < MAX_OSC {
                            self.payload.push(byte);
                        } else {
                            self.overflow = true;
                        }
                        Scan::Osc
                    }
                },
            };
        }
    }
}

pub(super) struct Modes {
    parser: vte::Parser,
    osc: OscScanner,
    log: Log,
}

impl Modes {
    pub(super) fn new(rows: u16) -> Self {
        Modes {
            parser: vte::Parser::new(),
            osc: OscScanner::default(),
            log: Log {
                rows,
                ..Log::default()
            },
        }
    }

    pub(super) fn process(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.log, bytes);
        self.osc.process(bytes, &mut self.log);
    }

    pub(super) fn resize(&mut self, rows: u16) {
        self.log.rows = rows;
        self.log.regions = [None; 2];
    }

    pub(super) fn set_alternate(&mut self, alternate: bool) {
        self.log.switch_screen(alternate);
    }

    pub(super) fn region(&self, alternate: bool) -> Option<String> {
        self.log.regions[usize::from(alternate)]
            .map(|(top, bottom)| format!("\x1b[{top};{bottom}r"))
    }

    pub(super) fn regions(&self) -> String {
        let mut text = self.region(false).unwrap_or_default();
        if let Some(region) = self.region(true) {
            text.push_str("\x1b[?1049h");
            text.push_str(&region);
        }
        text
    }

    pub(super) fn origin(&self) -> bool {
        let mut origin = false;
        for (slot, bytes) in &self.log.changes {
            match slot {
                Slot::Dec(6) => origin = bytes.ends_with('h'),
                Slot::SoftReset => origin = false,
                _ => {}
            }
        }
        origin
    }

    pub(super) fn drawing(&self) -> Vec<u8> {
        let mut insert = false;
        let mut newline = false;
        let mut wrap = true;
        let mut charsets: Vec<&str> = Vec::new();
        for (slot, bytes) in &self.log.changes {
            let on = bytes.ends_with('h');
            match slot {
                Slot::Ansi(4) => insert = on,
                Slot::Ansi(20) => newline = on,
                Slot::Dec(7) => wrap = on,
                Slot::Charset(_) | Slot::Shift => charsets.push(bytes),
                Slot::SoftReset => {
                    insert = false;
                    wrap = true;
                    charsets.clear();
                }
                _ => {}
            }
        }
        let set = |on: bool| if on { 'h' } else { 'l' };
        let mut text = format!(
            "\x1b[4{}\x1b[20{}\x1b[?7{}",
            set(insert),
            set(newline),
            set(wrap)
        );
        text.push_str(&charsets.concat());
        text.into_bytes()
    }

    pub(super) fn formatted(&self) -> Vec<u8> {
        let mut text = String::new();
        for (_, bytes) in &self.log.changes {
            text.push_str(bytes);
        }
        text.into_bytes()
    }

    pub(super) fn title(&self) -> Option<&str> {
        Some(self.log.program_title.text.as_str()).filter(|title| !title.is_empty())
    }

    pub(super) fn title_changed(&mut self) -> bool {
        std::mem::take(&mut self.log.program_title.changed)
    }

    pub(super) fn program_title(&self) -> ProgramTitle {
        self.log.program_title.clone()
    }

    pub(super) fn restore_program_title(&mut self, title: ProgramTitle) {
        self.log.program_title = title;
    }

    pub(super) fn take_notifications(&mut self) -> Vec<Notification> {
        std::mem::take(&mut self.log.notifications)
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;

    use super::*;

    fn formatted(bytes: &[u8]) -> String {
        let mut modes = Modes::new(24);
        modes.process(bytes);
        String::from_utf8(modes.formatted()).unwrap()
    }

    #[test]
    fn keeps_the_last_change_of_each_mode_in_order() {
        assert_eq!(
            formatted(b"\x1b[?1003h\x1b[?1000;1006h\x1b[?1003l\x1b[?1000h"),
            "\x1b[?1006h\x1b[?1003l\x1b[?1000h"
        );
    }

    #[test]
    fn ignores_screen_switches_and_reads_split_sequences() {
        let mut modes = Modes::new(24);
        modes.process(b"\x1b[?1049;20");
        modes.process(b"04h\x1b=\x1b]2;ti");
        modes.process(b"tle\x1b\\");
        assert_eq!(
            String::from_utf8(modes.formatted()).unwrap(),
            "\x1b[?2004h\x1b=\x1b]2;title\x07"
        );
    }

    #[test]
    fn resets_are_remembered() {
        assert_eq!(formatted(b"\x1b[?1h\x1b[!p"), "\x1b[?1h\x1b[!p");
        assert_eq!(
            formatted(b"\x1b[?1h\x1b[!p\x1b[?25l"),
            "\x1b[?1h\x1b[!p\x1b[?25l"
        );
    }

    #[test]
    fn a_full_reset_keeps_colors_and_the_title() {
        assert_eq!(
            formatted(b"\x1b]2;t\x07\x1b[4h\x1b[5 q\x1b]11;black\x07\x1b[?1h\x1bc\x1b[?25l"),
            "\x1b[?25l\x1b]2;t\x07\x1b]11;black\x07"
        );
    }

    #[test]
    fn ansi_modes_and_the_cursor_style_are_kept() {
        assert_eq!(
            formatted(b"\x1b[4h\x1b[20h\x1b[4l\x1b[5 q\x1b[2 q"),
            "\x1b[20h\x1b[4l\x1b[2 q"
        );
    }

    #[test]
    fn colors_are_kept_without_queries() {
        assert_eq!(
            formatted(b"\x1b]4;1;red;2;blue;3;?\x07\x1b]104;1\x07"),
            "\x1b]4;2;blue\x07\x1b]104;1\x07"
        );
        assert_eq!(
            formatted(b"\x1b]4;1;red\x07\x1b]104\x07\x1b]4;2;blue\x07"),
            "\x1b]104\x07\x1b]4;2;blue\x07"
        );
        assert_eq!(
            formatted(b"\x1b]10;red;blue\x07\x1b]11;?\x07\x1b]112\x07"),
            "\x1b]10;red\x07\x1b]11;blue\x07\x1b]112\x07"
        );
    }

    #[test]
    fn long_palettes_are_kept_whole() {
        let mut sequence = String::from("\x1b]4");
        for index in 0..16 {
            let _ = write!(sequence, ";{index};#0000{index:02}");
        }
        sequence.push('\x07');
        let kept = formatted(sequence.as_bytes());
        assert!(kept.ends_with("\x1b]4;15;#000015\x07"), "{kept:?}");
    }

    #[test]
    fn titles_with_semicolons_are_kept_whole() {
        assert_eq!(formatted(b"\x1b]0;a;b;c\x07"), "\x1b]0;a;b;c\x07");
    }

    fn drawing(bytes: &[u8]) -> String {
        let mut modes = Modes::new(24);
        modes.process(bytes);
        String::from_utf8(modes.drawing()).unwrap()
    }

    fn region(bytes: &[u8]) -> Option<String> {
        let mut modes = Modes::new(24);
        modes.process(bytes);
        modes.region(false)
    }

    fn alternate_region(bytes: &[u8]) -> Option<String> {
        let mut modes = Modes::new(24);
        modes.process(bytes);
        modes.region(true)
    }

    #[test]
    fn drawing_modes_follow_the_last_change_and_soft_resets() {
        assert_eq!(drawing(b""), "\x1b[4l\x1b[20l\x1b[?7h");
        assert_eq!(
            drawing(b"\x1b[4h\x1b[20h\x1b[?7l\x1b[?6h"),
            "\x1b[4h\x1b[20h\x1b[?7l"
        );
        assert_eq!(
            drawing(b"\x1b[4h\x1b[20h\x1b[?7l\x1b[!p"),
            "\x1b[4l\x1b[20h\x1b[?7h"
        );
        assert_eq!(drawing(b"\x1b[!p\x1b[4h"), "\x1b[4h\x1b[20l\x1b[?7h");
    }

    #[test]
    fn charsets_are_kept_and_restored_after_drawing() {
        assert_eq!(
            formatted(b"\x1b(0\x1b)A\x0e\x1b(B\x1bn"),
            "\x1b)A\x1b(B\x1bn"
        );
        assert_eq!(formatted(b"\x1b(0\x0e\x1bc"), "");
        assert_eq!(drawing(b"\x1b)0\x0e"), "\x1b[4l\x1b[20l\x1b[?7h\x1b)0\x0e");
        assert_eq!(drawing(b"\x1b(0\x1b[!p"), "\x1b[4l\x1b[20l\x1b[?7h");
    }

    #[test]
    fn origin_follows_the_last_change_and_soft_resets() {
        let origin = |bytes: &[u8]| {
            let mut modes = Modes::new(24);
            modes.process(bytes);
            modes.origin()
        };
        assert!(!origin(b""));
        assert!(origin(b"\x1b[?6h"));
        assert!(!origin(b"\x1b[?6h\x1b[!p"));
        assert!(!origin(b"\x1b[?6h\x1bc"));
    }

    #[test]
    fn the_main_screen_scroll_region_is_kept() {
        assert_eq!(region(b"\x1b[2;20r").as_deref(), Some("\x1b[2;20r"));
        assert_eq!(region(b"\x1b[5r").as_deref(), Some("\x1b[5;24r"));
        assert_eq!(region(b"\x1b[;10r").as_deref(), Some("\x1b[1;10r"));
        assert_eq!(region(b"\x1b[3;99r").as_deref(), Some("\x1b[3;24r"));
        assert_eq!(region(b"\x1b[2;20r\x1b[r"), None);
        assert_eq!(region(b"\x1b[2;20r\x1b[1;24r"), None);
        assert_eq!(
            region(b"\x1b[2;20r\x1b[9;9r").as_deref(),
            Some("\x1b[2;20r")
        );
        assert_eq!(region(b"\x1b[2;20r\x1b[30r").as_deref(), Some("\x1b[2;20r"));
        assert_eq!(region(b"\x1b[2;20r\x1b[!p"), None);
        assert_eq!(region(b"\x1b[2;20r\x1bc"), None);
    }

    #[test]
    fn the_alternate_screen_keeps_its_own_scroll_region() {
        assert_eq!(
            region(b"\x1b[2;20r\x1b[?1049h\x1b[5;10r\x1b[!p\x1b[r\x1b[?1049l").as_deref(),
            Some("\x1b[2;20r")
        );
        assert_eq!(region(b"\x1b[?1047h\x1b[5;10r\x1b[?1047l"), None);
        assert_eq!(
            region(b"\x1b[2;20r\x1b[?1048h\x1b[?1048l").as_deref(),
            Some("\x1b[2;20r")
        );
        assert_eq!(
            alternate_region(b"\x1b[2;20r\x1b[?1049h\x1b[5;10r").as_deref(),
            Some("\x1b[5;10r")
        );
        assert_eq!(alternate_region(b"\x1b[?1049h\x1b[5;10r\x1b[!p"), None);
        assert_eq!(alternate_region(b"\x1b[?1049h\x1b[5;10r\x1bc"), None);
        assert_eq!(
            alternate_region(b"\x1b[?1049h\x1b[5;10r\x1b[?1049l\x1b[?1049h"),
            None
        );
    }

    #[test]
    fn regions_move_to_the_next_process() {
        let mut first = Modes::new(24);
        first.process(b"\x1b[2;20r\x1b[?1049h\x1b[5;10r");
        let mut next = Modes::new(24);
        next.process(first.regions().as_bytes());
        next.set_alternate(true);

        assert_eq!(next.region(false), first.region(false));
        assert_eq!(next.region(true), first.region(true));
        next.set_alternate(false);
        assert_eq!(next.region(true), None);
    }

    #[test]
    fn resizing_drops_the_scroll_regions() {
        let mut modes = Modes::new(24);
        modes.process(b"\x1b[2;20r\x1b[?1049h\x1b[5;10r");
        modes.resize(30);
        assert_eq!(modes.region(false), None);
        assert_eq!(modes.region(true), None);
        modes.process(b"\x1b[3r");
        assert_eq!(modes.region(true).as_deref(), Some("\x1b[3;30r"));
    }

    fn notifications(bytes: &[u8]) -> Vec<Notification> {
        let mut modes = Modes::new(24);
        modes.process(bytes);
        modes.take_notifications()
    }

    fn notification(title: Option<&str>, body: &str) -> Notification {
        Notification {
            title: title.map(str::to_string),
            body: body.to_string(),
        }
    }

    #[test]
    fn notifications_come_in_both_formats() {
        assert_eq!(
            notifications(b"\x1b]9;Build done\x07"),
            [notification(None, "Build done")]
        );
        assert_eq!(
            notifications(b"\x1b]777;notify;Claude Code;Waiting; for you\x1b\\"),
            [notification(Some("Claude Code"), "Waiting; for you")]
        );
        assert_eq!(
            notifications(b"\x1b]777;notify;;Only a body\x07\x1b]777;notify;Only a title\x07"),
            [
                notification(None, "Only a body"),
                notification(Some("Only a title"), "")
            ]
        );
    }

    #[test]
    fn progress_and_other_codes_are_not_notifications() {
        let bytes =
            b"\x1b]9;4;1;50\x07\x1b]9;4;0;\x07\x1b]9;\x07\x1b]777;preexec\x07\x1b]777;notify;;\x07";
        assert!(notifications(bytes).is_empty());
    }

    #[test]
    fn a_notification_split_across_reads_arrives_once_whole() {
        let mut modes = Modes::new(24);
        modes.process(b"\x1b]777;notify;Ti");
        assert!(modes.take_notifications().is_empty());
        modes.process(b"tle;Body\x07");
        assert_eq!(
            modes.take_notifications(),
            [notification(Some("Title"), "Body")]
        );
        assert!(modes.take_notifications().is_empty());
    }

    #[test]
    fn a_program_title_lasts_until_its_command_ends() {
        let mut modes = Modes::new(24);
        modes.process(b"\x1b]2;shell\x07");
        assert_eq!(modes.title(), Some("shell"));
        assert!(modes.title_changed());
        modes.process(b"\x1b]2;shell\x07");
        assert!(!modes.title_changed());
        modes.process(b"\x1b]133;C\x07\x1b]0;vim; notes\x07");
        assert_eq!(modes.title(), Some("vim; notes"));
        modes.process(b"\x1b]133;D\x07");
        assert_eq!(modes.title(), None);
        assert!(modes.title_changed());
    }
}

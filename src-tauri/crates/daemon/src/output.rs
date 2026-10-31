use platform::Size;
use protocol::HISTORY_LINES;

use super::modes::{Modes, ProgramTitle};
use super::notifications::Notification;

const HISTORY_BYTES: usize = 16 * 1024 * 1024;

const HIDE_CURSOR: &[u8] = b"\x1b[?25l";
pub(super) const RESET_AFTER_HISTORY: &[u8] = b"\x1b[?1049l\x1b[!p\x1b[0m\x1b[?25h\
    \x1b[?9l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?1015l\x1b[?1016l\
    \x1b[?1004l\x1b[?2004l\x1b[?1l\x1b>\r\n";

const BEFORE_DRAWING: &[u8] = b"\x1b[r\x1b[?6l\x1b[?7h\x1b[4l\x1b[20l\x1b(B\x0f";

fn newlines(bytes: &[u8]) -> usize {
    bytes.iter().filter(|byte| **byte == b'\n').count()
}

pub(super) type Sink = Box<dyn FnMut(&[u8]) + Send>;

pub(super) enum Signal {
    Title(Option<String>),
    Notification(Notification),
}

pub(super) type Signals = Box<dyn FnMut(Signal) + Send>;

pub(super) struct Output {
    pub(super) parser: vt100::Parser,
    modes: Modes,
    sink: Sink,
    signals: Signals,
    history: Vec<u8>,
    lines: usize,
    dirty: bool,
    muted: bool,
}

impl Output {
    pub(super) fn new(size: Size, sink: Sink, signals: Signals) -> Self {
        Output {
            parser: vt100::Parser::new(size.rows, size.cols, 0),
            modes: Modes::new(size.rows),
            sink,
            signals,
            history: Vec::new(),
            lines: 0,
            dirty: false,
            muted: false,
        }
    }

    pub(super) fn alternate(&self) -> bool {
        self.parser.screen().alternate_screen()
    }

    pub(super) fn stop(&mut self) {
        self.emit(HIDE_CURSOR);
        self.mute();
    }

    pub(super) fn emit(&mut self, batch: &[u8]) {
        if self.muted {
            return;
        }
        self.feed(batch);
        for notification in self.modes.take_notifications() {
            (self.signals)(Signal::Notification(notification));
        }
        (self.sink)(batch);
    }

    pub(super) fn consume(&mut self, batch: &[u8]) {
        self.feed(batch);
        self.modes.take_notifications();
    }

    fn feed(&mut self, batch: &[u8]) {
        let was_alternate = self.alternate();
        self.parser.process(batch);
        self.modes.process(batch);
        if !was_alternate || !self.alternate() {
            self.record(batch);
        }
        if self.modes.title_changed() {
            let title = self.title().map(str::to_string);
            (self.signals)(Signal::Title(title));
        }
    }

    fn record(&mut self, batch: &[u8]) {
        self.history.extend_from_slice(batch);
        self.lines += newlines(batch);
        self.dirty = true;
        self.trim();
    }

    pub(super) fn set_history(&mut self, history: Vec<u8>) {
        self.lines = newlines(&history);
        self.history = history;
        self.dirty = true;
        self.trim();
    }

    pub(super) fn resize(&mut self, size: Size) {
        if self.parser.screen().size() != (size.rows, size.cols) {
            self.parser.screen_mut().set_size(size.rows, size.cols);
            self.modes.resize(size.rows);
        }
    }

    pub(super) fn mute(&mut self) {
        self.muted = true;
    }

    pub(super) fn title(&self) -> Option<&str> {
        self.modes.title()
    }

    pub(super) fn program_title(&self) -> ProgramTitle {
        self.modes.program_title()
    }

    pub(super) fn restore_program_title(&mut self, title: ProgramTitle) {
        self.modes.restore_program_title(title);
    }

    pub(super) fn history(&self) -> &[u8] {
        &self.history
    }

    pub(super) fn take_dirty_history(&mut self) -> Option<Vec<u8>> {
        std::mem::take(&mut self.dirty).then(|| self.history.clone())
    }

    pub(super) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn trim(&mut self) {
        if self.lines > HISTORY_LINES + HISTORY_LINES / 4 {
            let excess = self.lines - HISTORY_LINES;
            let cut = self
                .history
                .iter()
                .enumerate()
                .filter(|(_, byte)| **byte == b'\n')
                .nth(excess - 1)
                .map_or(0, |(at, _)| at + 1);
            self.history.drain(..cut);
            self.lines = HISTORY_LINES;
        }
        if self.history.len() > HISTORY_BYTES + HISTORY_BYTES / 4 {
            let cut = self.history.len() - HISTORY_BYTES;
            let cut = self.history[cut..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(cut, |at| cut + at + 1);
            self.lines -= newlines(&self.history[..cut]);
            self.history.drain(..cut);
        }
    }

    pub(super) fn snapshot(&self) -> Vec<u8> {
        let mut data = self.history.clone();
        let alternate = self.alternate();
        if alternate {
            data.extend_from_slice(b"\x1b[?1049h");
        }
        let screen = self.parser.screen();
        data.extend_from_slice(&self.modes.formatted());
        data.extend_from_slice(BEFORE_DRAWING);
        data.extend_from_slice(&screen.contents_formatted());
        if let Some(region) = self.modes.region(alternate) {
            data.extend_from_slice(region.as_bytes());
            data.extend_from_slice(&screen.cursor_state_formatted());
            data.extend_from_slice(&screen.attributes_formatted());
        }
        data.extend_from_slice(&self.modes.drawing());
        if self.modes.origin() {
            data.extend_from_slice(b"\x1b[?6h");
        }
        data
    }

    pub(super) fn modes(&self) -> String {
        let mut modes = String::from_utf8_lossy(&self.modes.formatted()).into_owned();
        modes.push_str(&self.modes.regions());
        modes
    }

    pub(super) fn restore_modes(&mut self, modes: &str) {
        self.modes.process(modes.as_bytes());
        self.modes.set_alternate(self.alternate());
    }

    pub(super) fn screen(&self) -> String {
        String::from_utf8_lossy(&self.parser.screen().state_formatted()).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use parking_lot::Mutex;

    use super::*;

    fn quiet() -> Output {
        Output::new(Size::new(24, 80), Box::new(|_| {}), Box::new(|_| {}))
    }

    #[test]
    fn a_stopped_output_hides_the_cursor_and_ignores_late_bytes() {
        let mut output = quiet();
        output.stop();
        output.emit(b"\x1b[?25hLATE");
        let screen = output.parser.screen();
        assert!(screen.hide_cursor());
        assert!(!screen.contents().contains("LATE"));
    }

    #[derive(Default)]
    struct Signalled {
        titles: Vec<Option<String>>,
        notifications: Vec<Notification>,
    }

    fn signalling() -> (Output, Arc<Mutex<Signalled>>) {
        let signalled = Arc::new(Mutex::new(Signalled::default()));
        let store = Arc::clone(&signalled);
        let output = Output::new(
            Size::new(24, 80),
            Box::new(|_| {}),
            Box::new(move |signal| {
                let mut store = store.lock();
                match signal {
                    Signal::Title(title) => store.titles.push(title),
                    Signal::Notification(notification) => store.notifications.push(notification),
                }
            }),
        );
        (output, signalled)
    }

    #[test]
    fn live_output_notifies_and_replayed_output_does_not() {
        let (mut output, signalled) = signalling();
        output.consume(b"\x1b]9;from history\x07");
        output.emit(b"\x1b]777;notify;Live;now\x07");
        output.consume(b"\x1b]9;replayed again\x07");

        let notifications = &signalled.lock().notifications;
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].title.as_deref(), Some("Live"));
        assert_eq!(notifications[0].body, "now");
    }

    #[test]
    fn title_changes_are_signalled_from_live_and_replayed_output() {
        let (mut output, signalled) = signalling();
        output.consume(b"\x1b]2;from history\x07");
        output.emit(b"\x1b]2;from history\x07plain output");
        output.emit(b"\x1b]133;C\x07\x1b]0;htop\x07\x1b]133;D\x07");

        let titles = &signalled.lock().titles;
        assert_eq!(titles, &[Some("from history".to_string()), None]);
    }

    #[test]
    fn a_muted_output_stops_signalling() {
        let (mut output, signalled) = signalling();
        output.mute();
        output.emit(b"\x1b]2;title\x07\x1b]9;after the restart began\x07");
        let signalled = signalled.lock();
        assert_eq!(signalled.titles, []);
        assert_eq!(signalled.notifications, []);
    }

    fn replayed(bytes: &[u8]) -> vt100::Parser {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(bytes);
        parser
    }

    #[test]
    fn replayed_history_does_not_leave_input_modes_on() {
        let mut output = quiet();
        output.emit(b"\x1b[?1003h\x1b[?1006h\x1b[?2004h\x1b[?1h\x1b[?1049h");
        output.emit(RESET_AFTER_HISTORY);
        let screen = output.parser.screen();
        assert_eq!(screen.mouse_protocol_mode(), vt100::MouseProtocolMode::None);
        assert!(!screen.bracketed_paste());
        assert!(!screen.application_cursor());
        assert!(!screen.alternate_screen());
    }

    #[test]
    fn history_skips_full_screen_redraws_but_keeps_what_surrounds_them() {
        let mut output = quiet();
        output.emit(b"before\r\n");
        output.emit(b"\x1b[?1049hFULL");
        output.emit(b"REDRAW");
        output.emit(b"LAST\x1b[?1049lafter\r\n");

        let history = String::from_utf8_lossy(&output.history).into_owned();
        assert!(history.contains("before"));
        assert!(history.contains("FULL"));
        assert!(!history.contains("REDRAW"));
        assert!(history.contains("after"));
        assert!(output.snapshot().starts_with(&output.history));
    }

    #[test]
    fn the_screen_survives_a_narrower_window() {
        let mut output = Output::new(Size::new(10, 40), Box::new(|_| {}), Box::new(|_| {}));
        for number in 0..30 {
            output.emit(format!("output line {number} with some words\r\n").as_bytes());
        }
        output.emit(b"+--------------------------------------+\r\n| > prompt                             |\r\n+--------------------------------------+");
        output.resize(Size::new(10, 30));
        output.emit(b"\r\x1b[2A\x1b[0J+----------------------------+\r\n| > prompt                   |\r\n+----------------------------+");

        let mut window = vt100::Parser::new(10, 30, 0);
        window.process(&output.snapshot());
        let mut history_only = vt100::Parser::new(10, 30, 0);
        history_only.process(&output.history);

        let expected = output.parser.screen().contents();
        assert_eq!(window.screen().contents(), expected);
        assert_eq!(
            window.screen().cursor_position(),
            output.parser.screen().cursor_position()
        );
        assert_ne!(history_only.screen().contents(), expected);
    }

    #[test]
    fn the_scroll_region_survives_the_snapshot() {
        let mut output = quiet();
        for number in 0..30 {
            output.emit(format!("line {number}\r\n").as_bytes());
        }
        output.emit(b"\x1b[2;20r\x1b[20;1H\x1b[31mred");
        let mut window = replayed(&output.snapshot());

        let more = b"\r\ninside\r\nthe region\x1b[24;1Hbelow";
        output.emit(more);
        window.process(more);

        let expected = output.parser.screen();
        assert_eq!(
            window.screen().contents_formatted(),
            expected.contents_formatted()
        );
        assert_eq!(
            window.screen().cursor_position(),
            expected.cursor_position()
        );
    }

    #[test]
    fn a_full_screen_program_survives_the_snapshot() {
        let mut output = quiet();
        output.emit(b"shell\r\n");
        output.emit(b"\x1b[?1049h\x1b[?7l\x1b[4h");
        output.emit(b"\x1b[?7h\x1b[4l\x1b[3;10r\x1b[5;1H");
        output.emit(&[b'x'; 100]);
        output.emit(b"\x1b[10;1H\x1b[32mgreen");
        let mut window = replayed(&output.snapshot());

        let more = b"\r\nnext\r\nlines\x1b[20;1Hbelow";
        output.emit(more);
        window.process(more);

        let expected = output.parser.screen();
        assert_eq!(
            window.screen().contents_formatted(),
            expected.contents_formatted()
        );
        assert_eq!(
            window.screen().cursor_position(),
            expected.cursor_position()
        );
    }

    #[test]
    fn the_scroll_region_moves_to_the_next_process() {
        let mut first = quiet();
        first.emit(b"\x1b[2;20r\x1b[?1049h\x1b[5;10r");
        let mut next = quiet();
        next.parser.process(b"\x1b[?1049h");
        next.restore_modes(&first.modes());

        assert_eq!(next.modes(), first.modes());
        assert_eq!(next.modes(), "\x1b[2;20r\x1b[?1049h\x1b[5;10r");
    }

    #[test]
    fn snapshot_inside_a_full_screen_program_redraws_its_screen() {
        let mut output = quiet();
        output.emit(b"shell\r\n");
        output.emit(b"\x1b[?1049h");
        output.emit(b"\x1b[5;1HCURRENT");

        let snapshot = String::from_utf8_lossy(&output.snapshot()).into_owned();
        let tail = &snapshot[output.history.len()..];
        assert!(tail.starts_with("\x1b[?1049h"));
        assert!(tail.contains("CURRENT"));
    }

    #[test]
    fn history_keeps_the_last_lines() {
        let mut output = quiet();
        for number in 0..3 * HISTORY_LINES {
            output.emit(format!("line {number}\r\n").as_bytes());
        }
        let kept = newlines(&output.history);
        assert!((HISTORY_LINES..=HISTORY_LINES + HISTORY_LINES / 4).contains(&kept));
        assert_eq!(kept, output.lines);
        assert!(output.history.starts_with(b"line "));
        assert!(
            output
                .history
                .ends_with(format!("line {}\r\n", 3 * HISTORY_LINES - 1).as_bytes())
        );
    }

    #[test]
    fn history_without_newlines_is_still_bounded() {
        let mut output = quiet();
        let chunk = vec![b'x'; 1024 * 1024];
        for _ in 0..40 {
            output.emit(&chunk);
        }
        assert!(output.history.len() <= HISTORY_BYTES + HISTORY_BYTES / 4);
    }

    #[test]
    fn trimming_by_size_keeps_the_line_count() {
        let mut output = quiet();
        let line = [vec![b'x'; 4096], b"\r\n".to_vec()].concat();
        for _ in 0..6000 {
            output.emit(&line);
        }
        assert_eq!(output.lines, newlines(&output.history));
        assert!(output.history.len() <= HISTORY_BYTES + HISTORY_BYTES / 4);
    }

    #[test]
    fn modes_changed_inside_a_full_screen_program_survive_the_snapshot() {
        let mut output = quiet();
        output.emit(b"\x1b[?1049h\x1b[?1000h");
        output.emit(b"\x1b[?1000l");
        output.emit(b"\x1b[?1049l");

        let history = replayed(&output.history);
        let snapshot = replayed(&output.snapshot());
        assert_eq!(
            history.screen().mouse_protocol_mode(),
            vt100::MouseProtocolMode::PressRelease
        );
        assert_eq!(
            snapshot.screen().mouse_protocol_mode(),
            vt100::MouseProtocolMode::None
        );
    }

    #[test]
    fn modes_survive_trimming() {
        let mut output = quiet();
        output.emit(b"\x1b[?2004h");
        for number in 0..3 * HISTORY_LINES {
            output.emit(format!("line {number}\r\n").as_bytes());
        }

        assert!(!replayed(&output.history).screen().bracketed_paste());
        assert!(replayed(&output.snapshot()).screen().bracketed_paste());
    }

    #[test]
    fn modes_move_to_the_next_process() {
        let mut first = quiet();
        first.emit(b"\x1b[?1004h\x1b[?25l");
        let mut next = quiet();
        next.restore_modes(&first.modes());

        assert_eq!(next.modes(), first.modes());
        assert_eq!(next.modes(), "\x1b[?1004h\x1b[?25l");
    }

    fn moved(first: &Output) -> Output {
        let mut next = quiet();
        next.restore_modes(&first.modes());
        next.restore_program_title(first.program_title());
        next
    }

    #[test]
    fn a_finished_program_title_does_not_come_back_in_the_next_process() {
        let mut first = quiet();
        first.emit(b"\x1b]133;C\x07\x1b]2;vim\x07\x1b]133;D\x07");
        assert_eq!(first.title(), None);
        assert_eq!(moved(&first).title(), None);
    }

    #[test]
    fn a_running_program_title_moves_to_the_next_process_and_ends_there() {
        let mut first = quiet();
        first.emit(b"\x1b]133;C\x07\x1b]2;htop\x07");
        let mut next = moved(&first);
        assert_eq!(next.title(), Some("htop"));
        next.emit(b"\x1b]133;D\x07");
        assert_eq!(next.title(), None);
    }
}

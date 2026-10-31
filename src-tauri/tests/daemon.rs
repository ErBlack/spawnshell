#![cfg(unix)]

use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use platform::ipc::{self, Lock, Stream};
use platform::process::{self, Pid};
use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_spawnshell");
#[cfg(target_os = "macos")]
const DATA_ROOT: &str = "Library/Application Support";
#[cfg(not(target_os = "macos"))]
const DATA_ROOT: &str = ".local/share";
const IDENTIFIER: &str = "com.erblack.spawnshell-terminal";
const PATIENCE: Duration = Duration::from_secs(10);
const HIDE_CURSOR: &str = "\x1b[?25l";
const SHOW_CURSOR: &str = "\x1b[?25h";

fn eventually(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn timed_out(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

fn succeeded(output: &Output) -> bool {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

enum Frame {
    Message(Value),
    Output(Vec<u8>),
    Snapshot(Vec<u8>),
}

fn tagged(kind: u8, tab: &str, data: &[u8]) -> Vec<u8> {
    let mut payload = vec![kind];
    payload.extend_from_slice(&u16::try_from(tab.len()).unwrap().to_be_bytes());
    payload.extend_from_slice(tab.as_bytes());
    payload.extend_from_slice(data);
    payload
}

fn send_raw(stream: &mut Stream, body: &[u8]) {
    let mut buffer = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
    buffer.extend_from_slice(body);
    stream.write_all(&buffer).unwrap();
}

fn read_frame(reader: &mut impl Read) -> io::Result<Frame> {
    let mut header = [0u8; 4];
    reader.read_exact(&mut header)?;
    let mut body = vec![0u8; u32::from_be_bytes(header) as usize];
    reader.read_exact(&mut body)?;
    let payload = body.split_off(1);
    if body[0] == 0 {
        return Ok(Frame::Message(serde_json::from_slice(&payload).unwrap()));
    }
    let length = u16::from_be_bytes([payload[0], payload[1]]) as usize;
    let data = payload[2 + length..].to_vec();
    match body[0] {
        1 => Ok(Frame::Output(data)),
        3 => Ok(Frame::Snapshot(data)),
        kind => panic!("unexpected frame kind {kind}"),
    }
}

struct Daemon {
    child: Child,
    socket: PathBuf,
}

impl Daemon {
    fn exited(&mut self) -> bool {
        eventually(Duration::from_secs(5), || {
            self.child.try_wait().unwrap().is_some()
        })
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if let Ok(mut client) = Client::connect(&self.socket, "cli", Duration::from_millis(200)) {
            let _ = client.try_call(&json!({"command": "stop"}));
        }
        if !self.exited() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

struct Env {
    home: PathBuf,
    socket: PathBuf,
}

impl Env {
    fn new(name: &str) -> Env {
        let base = std::env::temp_dir();
        let home = base.join(format!("spawnshell-it-{name}-{}", std::process::id()));
        let socket = base.join(format!("spawnshell-{name}-{}.sock", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        Env { home, socket }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(BIN);
        command
            .env("HOME", &self.home)
            .env("SPAWNSHELL_SOCKET", &self.socket)
            .env("SHELL", "/bin/sh")
            .env_remove("SPAWNSHELL_TAB")
            .current_dir(&self.home)
            .stdin(Stdio::null())
            .stdout(Stdio::null());
        command
    }

    fn spawnshell(&self, args: &[&str]) -> Output {
        self.command()
            .args(args)
            .stdout(Stdio::piped())
            .output()
            .unwrap()
    }

    fn start(&self) -> Daemon {
        self.start_with(&mut self.command())
    }

    fn start_with(&self, command: &mut Command) -> Daemon {
        Daemon {
            child: command.arg("__daemon").spawn().unwrap(),
            socket: self.socket.clone(),
        }
    }

    fn connect(&self, role: &str) -> Client {
        Client::connect(&self.socket, role, Duration::from_secs(5)).unwrap()
    }

    fn data_dir(&self) -> PathBuf {
        self.home.join(DATA_ROOT).join(IDENTIFIER)
    }

    fn session(&self) -> Value {
        let path = self.data_dir().join("session.json");
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
        ipc::remove_socket(&self.socket);
    }
}

struct Client {
    writer: Stream,
    reader: BufReader<Stream>,
    seq: u64,
    output: Vec<u8>,
    live: Vec<u8>,
}

impl Client {
    fn connect(socket: &Path, role: &str, patience: Duration) -> io::Result<Client> {
        let deadline = Instant::now() + patience;
        let writer = loop {
            match Stream::connect(socket) {
                Ok(stream) => break stream,
                Err(error) if Instant::now() > deadline => return Err(error),
                Err(_) => thread::sleep(Duration::from_millis(50)),
            }
        };
        let reader = BufReader::new(writer.try_clone()?);
        let mut client = Client {
            writer,
            reader,
            seq: 0,
            output: Vec::new(),
            live: Vec::new(),
        };
        client.try_call(&json!({"command": "hello", "role": role}))?;
        Ok(client)
    }

    fn try_call(&mut self, command: &Value) -> io::Result<Value> {
        self.seq += 1;
        let request = json!({"seq": self.seq, "command": command});
        let mut body = vec![0u8];
        body.extend_from_slice(&serde_json::to_vec(&request).unwrap());
        send_raw(&mut self.writer, &body);
        let deadline = Instant::now() + PATIENCE;
        loop {
            let message = self.next_message(deadline)?;
            if message["kind"] == "response" && message["seq"] == self.seq {
                return Ok(message["result"].clone());
            }
        }
    }

    fn next_frame(&mut self, deadline: Instant) -> io::Result<Frame> {
        let left = deadline
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1));
        self.reader.get_ref().set_read_timeout(Some(left))?;
        let frame = read_frame(&mut self.reader)?;
        if let Frame::Output(data) | Frame::Snapshot(data) = &frame {
            self.output.extend_from_slice(data);
        }
        if let Frame::Output(data) = &frame {
            self.live.extend_from_slice(data);
        }
        Ok(frame)
    }

    fn next_message(&mut self, deadline: Instant) -> io::Result<Value> {
        loop {
            if let Frame::Message(message) = self.next_frame(deadline)? {
                return Ok(message);
            }
        }
    }

    fn call(&mut self, command: &Value) -> Value {
        let result = self.try_call(command).unwrap();
        assert!(result.get("Ok").is_some(), "request failed: {result}");
        result["Ok"].clone()
    }

    fn type_text(&mut self, tab: &str, text: &str) {
        send_raw(&mut self.writer, &tagged(2, tab, text.as_bytes()));
    }

    fn expect(&mut self, needle: &str) -> String {
        let text = self.wait_for(needle);
        let at = text.find(needle).unwrap() + needle.len();
        text[at..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect()
    }

    fn wait_for(&mut self, needle: &str) -> String {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let text = String::from_utf8_lossy(&self.output).into_owned();
            if text.contains(needle) {
                return text;
            }
            match self.next_frame(deadline) {
                Ok(_) => {}
                Err(error) if timed_out(&error) => panic!("never saw {needle:?} in {text:?}"),
                Err(error) => panic!("connection failed: {error}"),
            }
        }
    }

    fn event(&mut self, name: &str) -> Value {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let message = self
                .next_message(deadline)
                .unwrap_or_else(|error| panic!("never got a {name} event: {error}"));
            if message["kind"] == "event" && message["event"]["event"] == name {
                return message["event"].clone();
            }
        }
    }

    fn tabs(&mut self) -> Vec<Value> {
        self.call(&json!({"command": "list"}))["tabs"]
            .as_array()
            .unwrap()
            .clone()
    }

    fn closed(&mut self) -> bool {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Err(error) = self.next_frame(deadline) {
                return !timed_out(&error);
            }
        }
    }
}

fn wait_dead(pid: Pid) -> bool {
    eventually(Duration::from_secs(5), || !process::exists(pid))
}

fn shell_pid(client: &mut Client, tab: &str, marker: &str) -> Pid {
    client.type_text(tab, &format!("echo {marker}':'$$\n"));
    serde_json::from_str(&client.expect(&format!("{marker}:"))).unwrap()
}

fn new_tab(client: &mut Client, pinned: bool) -> String {
    let tab = client.call(&json!({"command": "new", "pinned": pinned}))["tab"]
        .as_str()
        .unwrap()
        .to_string();
    client.call(&json!({"command": "attach", "tab": tab}));
    tab
}

#[test]
fn pinned_processes_survive_an_upgrade() {
    let env = Env::new("upgrade");
    let mut first = env.start();
    let mut gui = env.connect("gui");
    let tab = new_tab(&mut gui, true);
    let pid = shell_pid(&mut gui, &tab, "BEFORE");

    let upgrade = gui.try_call(&json!({"command": "upgrade", "exe": BIN}));
    assert!(
        upgrade.is_err() || gui.closed(),
        "old daemon kept the connection: {upgrade:?}"
    );
    assert!(first.exited(), "old daemon did not exit");
    assert!(process::exists(pid), "shell died during the upgrade");

    let mut gui = env.connect("gui");
    let tabs = gui.tabs();
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0]["id"], tab);
    assert_eq!(tabs[0]["alive"], true);
    gui.call(&json!({"command": "attach", "tab": tab}));
    assert_eq!(
        serde_json::from_str::<Pid>(&gui.expect("BEFORE:")).unwrap(),
        pid,
        "screen was not carried over"
    );
    assert_eq!(
        shell_pid(&mut gui, &tab, "AFTER"),
        pid,
        "a different shell answered"
    );

    gui.call(&json!({"command": "stop"}));
    assert!(wait_dead(pid), "stop left the shell running");
    let session = env.session();
    assert_eq!(session["tabs"][0]["id"], tab);
}

#[test]
fn quitting_the_gui_closes_unpinned_tabs_and_keeps_pinned_ones() {
    let env = Env::new("quit");
    let _daemon = env.start();
    let mut gui = env.connect("gui");
    let pinned = new_tab(&mut gui, true);
    let pinned_pid = shell_pid(&mut gui, &pinned, "PINNED");
    let loose = new_tab(&mut gui, false);
    let loose_pid = shell_pid(&mut gui, &loose, "LOOSE");
    drop(gui);

    assert!(wait_dead(loose_pid), "unpinned shell outlived the GUI");
    assert!(
        process::exists(pinned_pid),
        "pinned shell died with the GUI"
    );
    let mut cli = env.connect("cli");
    let tabs = cli.tabs();
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0]["id"], pinned);
    assert_eq!(tabs[0]["alive"], true);
    assert!(
        process::exists(pinned_pid),
        "pinned shell died after the GUI left"
    );
}

#[test]
fn closing_a_pinned_tab_unloads_it_unless_forced() {
    let env = Env::new("close");
    let _daemon = env.start();
    let mut gui = env.connect("gui");
    let pinned = new_tab(&mut gui, true);
    let loose = new_tab(&mut gui, false);
    let pid = shell_pid(&mut gui, &pinned, "PINNED");

    assert!(succeeded(&env.spawnshell(&["close", "--tab", &pinned])));
    assert!(wait_dead(pid), "unloading left the shell running");
    assert!(succeeded(&env.spawnshell(&["close", "--tab", &loose])));
    let tabs = gui.tabs();
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0]["id"], pinned);
    assert_eq!(tabs[0]["alive"], false);

    gui.call(&json!({"command": "restart", "tab": pinned}));
    assert_ne!(shell_pid(&mut gui, &pinned, "AGAIN"), pid);

    assert!(succeeded(
        &env.spawnshell(&["close", "--force", "--tab", &pinned])
    ));
    assert_eq!(gui.tabs(), Vec::<Value>::new());
}

#[test]
fn a_stopped_tab_hides_the_cursor_until_it_restarts() {
    let env = Env::new("cursor");
    let _daemon = env.start();
    let mut gui = env.connect("gui");
    let tab = new_tab(&mut gui, true);
    let pid = shell_pid(&mut gui, &tab, "FIRST");
    gui.type_text(&tab, "exit\n");
    assert!(wait_dead(pid));
    gui.wait_for(HIDE_CURSOR);

    gui.output.clear();
    gui.call(&json!({"command": "attach", "tab": tab}));
    gui.wait_for(HIDE_CURSOR);

    gui.output.clear();
    gui.call(&json!({"command": "restart", "tab": tab}));
    let pid = shell_pid(&mut gui, &tab, "SECOND");
    let text = String::from_utf8_lossy(&gui.output).into_owned();
    assert!(
        text.rfind(SHOW_CURSOR) > text.rfind(HIDE_CURSOR),
        "cursor left hidden: {text:?}"
    );

    gui.output.clear();
    gui.call(&json!({"command": "close", "tab": tab}));
    assert!(wait_dead(pid));
    gui.wait_for(HIDE_CURSOR);
}

#[cfg(target_os = "macos")]
#[test]
fn zsh_runs_with_the_shell_integration() {
    let env = Env::new("zsh");
    let _daemon = env.start_with(env.command().env("SHELL", "/bin/zsh"));
    let mut gui = env.connect("gui");
    new_tab(&mut gui, true);

    gui.wait_for("\x1b]133;D");
}

#[test]
fn a_failed_restart_leaves_the_cursor_hidden() {
    let env = Env::new("failed-restart");
    let shell = env.home.join("shell");
    std::os::unix::fs::symlink("/bin/sh", &shell).unwrap();
    let _daemon = env.start_with(env.command().env("SHELL", &shell));
    let mut gui = env.connect("gui");
    let tab = new_tab(&mut gui, true);
    let pid = shell_pid(&mut gui, &tab, "FIRST");
    gui.type_text(&tab, "exit\n");
    assert!(wait_dead(pid));
    assert!(
        eventually(PATIENCE, || gui.tabs()[0]["alive"] == false),
        "tab never stopped"
    );
    std::fs::remove_file(&shell).unwrap();

    let restart = gui
        .try_call(&json!({"command": "restart", "tab": tab}))
        .unwrap();
    assert!(restart.get("Ok").is_none(), "restart worked: {restart}");
    assert_eq!(gui.tabs()[0]["alive"], false);

    gui.output.clear();
    gui.call(&json!({"command": "attach", "tab": tab}));
    let text = gui.wait_for("couldn't start the shell");
    assert!(
        text.rfind(HIDE_CURSOR) > text.rfind(SHOW_CURSOR),
        "cursor came back: {text:?}"
    );
}

#[test]
fn a_pinned_tab_whose_shell_exits_stays_and_restarts() {
    let env = Env::new("restart");
    let _daemon = env.start();
    let mut gui = env.connect("gui");
    let tab = new_tab(&mut gui, true);
    let pid = shell_pid(&mut gui, &tab, "FIRST");
    gui.type_text(&tab, "exit\n");
    assert!(wait_dead(pid));

    assert!(
        eventually(Duration::from_secs(5), || gui.tabs()[0]["alive"] == false),
        "tab never stopped"
    );
    assert_eq!(gui.tabs().len(), 1, "stopped pinned tab was removed");

    gui.call(&json!({"command": "restart", "tab": tab}));
    assert_eq!(gui.tabs()[0]["alive"], true);
    let next = shell_pid(&mut gui, &tab, "SECOND");
    assert_ne!(next, pid);
}

#[test]
fn history_beyond_the_screen_survives_reconnects_and_restarts() {
    let env = Env::new("history");
    let first = env.start();
    let mut gui = env.connect("gui");
    let tab = new_tab(&mut gui, true);
    gui.type_text(&tab, "seq 1 300 | sed 's/^/L/'\n");
    gui.wait_for("L300");
    drop(gui);

    let mut gui = env.connect("gui");
    gui.call(&json!({"command": "attach", "tab": tab}));
    let text = gui.wait_for("L300");
    assert!(
        text.contains("L1\r\n"),
        "early lines were lost on reconnect"
    );
    drop(gui);
    drop(first);

    let _second = env.start();
    let mut gui = env.connect("gui");
    assert_eq!(gui.tabs()[0]["id"], tab);
    gui.call(&json!({"command": "attach", "tab": tab}));
    let text = gui.wait_for("L300");
    assert!(
        text.contains("L1\r\n"),
        "early lines were lost after a restart"
    );
}

#[test]
fn a_restarted_tab_replays_its_history_as_a_snapshot() {
    let env = Env::new("replay");
    let _daemon = env.start();
    let mut gui = env.connect("gui");
    let tab = new_tab(&mut gui, true);
    gui.type_text(&tab, "echo OLD$((6*7))\n");
    gui.wait_for("OLD42");

    gui.output.clear();
    gui.live.clear();
    gui.call(&json!({"command": "restart", "tab": tab}));
    shell_pid(&mut gui, &tab, "NEW");
    assert!(String::from_utf8_lossy(&gui.output).contains("OLD42"));
    assert!(
        !String::from_utf8_lossy(&gui.live).contains("OLD42"),
        "history was replayed as live output"
    );
}

#[test]
fn the_selected_pinned_tab_survives_a_restart() {
    let env = Env::new("selected");
    let first = env.start();
    let mut gui = env.connect("gui");
    new_tab(&mut gui, true);
    let selected = new_tab(&mut gui, true);
    gui.call(&json!({"command": "activate", "tab": selected}));
    let active: Vec<bool> = gui
        .tabs()
        .iter()
        .map(|tab| tab["active"].as_bool().unwrap())
        .collect();
    assert_eq!(active, [false, true]);
    drop(gui);
    drop(first);

    let _second = env.start();
    let mut gui = env.connect("gui");
    let tabs = gui.tabs();
    assert_eq!(tabs[1]["id"], selected);
    assert_eq!(tabs[1]["active"], true);

    let unpinned = new_tab(&mut gui, false);
    gui.call(&json!({"command": "activate", "tab": unpinned}));
    gui.call(&json!({"command": "save"}));
    assert!(env.session().get("selected").is_none());
}

#[test]
fn a_tab_title_set_from_the_cli_survives_restarts_and_resets() {
    let env = Env::new("rename");
    let first = env.start();
    let mut gui = env.connect("gui");
    let tab = new_tab(&mut gui, true);

    assert!(succeeded(
        &env.spawnshell(&["rename", " build ", "--tab", &tab])
    ));
    assert_eq!(gui.tabs()[0]["title"], "build");
    let output = env.spawnshell(&["list"]);
    assert!(String::from_utf8_lossy(&output.stdout).ends_with("\tbuild\t-\n"));
    assert_eq!(env.session()["tabs"][0]["title"], "build");

    let upgrade = gui.try_call(&json!({"command": "upgrade", "exe": BIN}));
    assert!(upgrade.is_err() || gui.closed());
    let mut gui = env.connect("gui");
    assert_eq!(gui.tabs()[0]["title"], "build", "lost in an upgrade");
    gui.call(&json!({"command": "stop"}));
    drop(gui);
    drop(first);

    let _second = env.start();
    let mut gui = env.connect("gui");
    assert_eq!(gui.tabs()[0]["title"], "build", "lost in a restart");

    assert!(succeeded(&env.spawnshell(&["rename", "  ", "--tab", &tab])));
    assert!(gui.tabs()[0]["title"].is_null());
    for rejected in [vec![], vec!["a\tb"]] {
        let mut args = vec!["rename", "--tab", &tab];
        args.extend(rejected);
        assert!(!env.spawnshell(&args).status.success(), "accepted {args:?}");
    }
}

#[test]
fn a_restore_command_set_from_the_cli_runs_when_the_tab_comes_back() {
    let env = Env::new("restore");
    let _daemon = env.start();
    let mut gui = env.connect("gui");
    let pinned = new_tab(&mut gui, true);
    let loose = new_tab(&mut gui, false);
    let restore = "echo RESTORED$((6*7))";

    assert!(succeeded(
        &env.spawnshell(&["restore", restore, "--tab", &pinned])
    ));
    let output = env.spawnshell(&["restore", "--tab", &pinned]);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("{restore}\n")
    );
    assert_eq!(env.session()["tabs"][0]["restore"], restore);

    gui.call(&json!({"command": "restart", "tab": pinned}));
    gui.wait_for("RESTORED42");

    assert!(succeeded(
        &env.spawnshell(&["restore", "", "--tab", &pinned])
    ));
    assert!(env.session()["tabs"][0]["restore"].is_null());
    for rejected in [vec!["echo x"], vec![]] {
        let mut args = vec!["restore", "--tab", &loose];
        args.extend(rejected);
        let output = env.spawnshell(&args);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("only pinned tabs"),
            "{args:?} printed {stderr:?}"
        );
    }
}

#[test]
fn an_unreadable_session_is_set_aside() {
    let env = Env::new("unreadable");
    let dir = env.data_dir();
    std::fs::create_dir_all(dir.join("history")).unwrap();
    std::fs::write(dir.join("session.json"), "{not json").unwrap();
    std::fs::write(dir.join("history").join("kept"), "old output").unwrap();
    let _daemon = env.start();
    let mut gui = env.connect("gui");

    let tab = new_tab(&mut gui, true);
    gui.call(&json!({"command": "save"}));

    assert_eq!(
        std::fs::read_to_string(dir.join("session.json.bad")).unwrap(),
        "{not json"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("history.bad").join("kept")).unwrap(),
        "old output"
    );
    assert_eq!(env.session()["tabs"][0]["id"], tab);
}

#[test]
fn an_unreadable_history_is_set_aside() {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    let env = Env::new("locked");
    let dir = env.data_dir();
    let history = dir.join("history");
    std::fs::create_dir_all(&history).unwrap();
    std::fs::write(dir.join("session.json"), r#"{"tabs":[{"id":"locked"}]}"#).unwrap();
    std::fs::write(history.join("locked"), "old output").unwrap();
    std::fs::set_permissions(history.join("locked"), Permissions::from_mode(0o000)).unwrap();
    let _daemon = env.start();
    let mut gui = env.connect("gui");

    gui.call(&json!({"command": "save"}));

    let aside = history.join("locked.bad");
    std::fs::set_permissions(&aside, Permissions::from_mode(0o600)).unwrap();
    assert_eq!(std::fs::read_to_string(&aside).unwrap(), "old output");
}

#[test]
fn an_unknown_request_gets_an_answer_instead_of_a_hangup() {
    let env = Env::new("unknown");
    let _daemon = env.start();
    let mut cli = env.connect("cli");

    let result = cli
        .try_call(&json!({"command": "from_the_future"}))
        .unwrap();
    assert!(
        result["Err"]
            .as_str()
            .is_some_and(|error| error.contains("unsupported request")),
        "{result}"
    );
    cli.tabs();
}

#[test]
fn stop_answers_everyone_before_the_daemon_exits() {
    let env = Env::new("stop");
    let mut daemon = env.start();
    let mut gui = env.connect("gui");

    assert!(succeeded(&env.spawnshell(&["stop"])));
    gui.event("stop");
    drop(gui);
    assert!(daemon.exited());
}

fn replace(path: &Path, text: &str) {
    let staging = path.with_extension("tmp");
    std::fs::write(&staging, text).unwrap();
    std::fs::rename(&staging, path).unwrap();
}

#[test]
fn settings_change_through_the_cli_and_the_file() {
    let env = Env::new("config");
    let path = env.home.join(".config/spawnshell/config.toml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "# mine\nfont_size = 14 # small\n").unwrap();
    let _daemon = env.start();
    let mut gui = env.connect("gui");

    assert!(succeeded(&env.spawnshell(&["config", "font_size", "18"])));
    assert_eq!(gui.event("config")["config"]["font_size"], 18);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# mine\nfont_size = 18 # small\n"
    );
    let output = env.spawnshell(&["config", "font_size"]);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "18\n");

    let output = env.spawnshell(&["config", "height", "500"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("height must be"));

    replace(&path, "font_size = 12\n");
    assert_eq!(gui.event("config")["config"]["font_size"], 12);

    replace(&path, "font_size = \n");
    let warning = gui.event("warning");
    assert!(warning["message"].as_str().unwrap().contains("not valid"));
    let output = env.spawnshell(&["config", "font_size"]);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "12\n");
}

#[test]
fn cli_needs_a_tab_outside_spawnshell_and_reports_a_missing_daemon() {
    let env = Env::new("cli");
    let output = env.spawnshell(&["list"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not running"));

    assert!(succeeded(&env.spawnshell(&["--help"])));

    let _daemon = env.start();
    env.connect("cli");
    let output = env.spawnshell(&["close"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--tab"));

    assert!(succeeded(&env.spawnshell(&["list"])));
}

#[test]
fn a_new_tab_becomes_active() {
    let env = Env::new("new-active");
    let _daemon = env.start();
    let mut gui = env.connect("gui");
    let first = new_tab(&mut gui, false);
    let second = new_tab(&mut gui, false);

    assert!(flagged(&mut gui, &second, "active"));
    assert!(!flagged(&mut gui, &first, "active"));
}

#[test]
fn the_cli_opens_a_new_tab_in_a_directory() {
    let env = Env::new("cwd");
    let _daemon = env.start();
    let mut gui = env.connect("gui");
    let project = env.home.join("project");
    std::fs::create_dir(&project).unwrap();

    let output = env.spawnshell(&["new", "project"]);
    assert!(succeeded(&output));
    let tab = String::from_utf8_lossy(&output.stdout).trim().to_string();
    gui.call(&json!({"command": "attach", "tab": tab}));
    gui.type_text(&tab, "echo CWD:$(pwd):\n");
    gui.wait_for(&format!(
        "CWD:{}:",
        project.canonicalize().unwrap().display()
    ));
}

#[test]
fn a_new_tab_needs_an_absolute_path_to_a_directory() {
    let env = Env::new("bad-cwd");
    let _daemon = env.start();
    let mut gui = env.connect("gui");
    std::fs::create_dir(env.home.join("project")).unwrap();
    std::fs::write(env.home.join("notes"), "").unwrap();

    for (path, error) in [
        ("missing", "No such file or directory"),
        ("notes", "is not a directory"),
    ] {
        let output = env.spawnshell(&["new", path]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "opened {path}");
        assert!(stderr.contains(error), "{path}: {stderr}");
    }
    let reply = gui
        .try_call(&json!({"command": "new", "pinned": false, "cwd": "project"}))
        .unwrap();
    assert_eq!(reply["Err"], "project is not an absolute path");
    let missing = env.home.join("missing");
    let reply = gui
        .try_call(&json!({"command": "new", "pinned": false, "cwd": missing}))
        .unwrap();
    assert_eq!(
        reply["Err"],
        format!("{} does not exist", missing.display())
    );
    assert_eq!(gui.tabs(), Vec::<Value>::new());
}

fn run(client: &mut Client, tab: &str, command: &str, marker: &str) {
    client.type_text(tab, &format!("{command}; echo {marker}':'\n"));
    client.wait_for(&format!("{marker}:"));
}

fn flagged(client: &mut Client, tab: &str, flag: &str) -> bool {
    client
        .tabs()
        .iter()
        .any(|info| info["id"] == tab && info[flag] == true)
}

fn notified(client: &mut Client, tab: &str) -> bool {
    flagged(client, tab, "notified")
}

#[test]
fn a_notification_marks_its_tab_until_it_is_seen() {
    let env = Env::new("notify");
    let _daemon = env.start();
    let mut gui = env.connect("gui");
    let first = new_tab(&mut gui, false);
    let second = new_tab(&mut gui, false);
    gui.call(&json!({"command": "activate", "tab": first}));
    gui.call(&json!({"command": "set_window_open", "open": true}));

    run(&mut gui, &first, r"printf '\033]9;in sight\007'", "active");
    run(
        &mut gui,
        &second,
        r"printf '\033]777;notify;Build;done\007'",
        "background",
    );
    assert!(eventually(PATIENCE, || notified(&mut gui, &second)));
    assert!(!notified(&mut gui, &first), "the tab in sight was notified");

    assert!(succeeded(&env.spawnshell(&["select", "--tab", &second])));
    assert!(!notified(&mut gui, &second));
    assert!(flagged(&mut gui, &second, "active"));

    gui.call(&json!({"command": "set_window_open", "open": false}));
    run(&mut gui, &second, r"printf '\033]9;hidden\007'", "hidden");
    assert!(eventually(PATIENCE, || notified(&mut gui, &second)));
    gui.call(&json!({"command": "set_window_open", "open": true}));
    assert!(!notified(&mut gui, &second));
}

#[test]
fn the_cli_notifies_from_a_tab_and_the_mark_survives_upgrades_and_restarts() {
    let env = Env::new("notify-cli");
    let first = env.start();
    let mut gui = env.connect("gui");
    let tab = new_tab(&mut gui, true);

    assert!(succeeded(&env.spawnshell(&[
        "notify", "--title", "Deploy", "finished", "--tab", &tab
    ])));
    assert!(notified(&mut gui, &tab));
    let output = env.spawnshell(&["list"]);
    assert!(String::from_utf8_lossy(&output.stdout).ends_with("\tnotified\n"));

    for rejected in [vec!["", "--tab", &tab], vec!["text", "--tab", "nope"]] {
        let mut args = vec!["notify"];
        args.extend(rejected);
        assert!(!env.spawnshell(&args).status.success(), "accepted {args:?}");
    }

    let upgrade = gui.try_call(&json!({"command": "upgrade", "exe": BIN}));
    assert!(upgrade.is_err() || gui.closed());
    let mut gui = env.connect("gui");
    assert!(notified(&mut gui, &tab), "lost in an upgrade");
    gui.call(&json!({"command": "stop"}));
    drop(gui);
    drop(first);

    let _second = env.start();
    let mut gui = env.connect("gui");
    assert!(notified(&mut gui, &tab), "lost in a restart");
}

#[test]
fn a_restarted_tab_does_not_repeat_its_notifications() {
    let env = Env::new("notify-restart");
    let _daemon = env.start();
    let mut gui = env.connect("gui");
    let tab = new_tab(&mut gui, true);
    run(&mut gui, &tab, r"printf '\033]9;once\007'", "once");
    assert!(eventually(PATIENCE, || notified(&mut gui, &tab)));
    gui.call(&json!({"command": "set_window_open", "open": true}));
    gui.call(&json!({"command": "activate", "tab": tab}));
    assert!(!notified(&mut gui, &tab));

    gui.call(&json!({"command": "set_window_open", "open": false}));
    gui.call(&json!({"command": "restart", "tab": tab}));
    run(&mut gui, &tab, "true", "restarted");
    let other = new_tab(&mut gui, false);
    run(&mut gui, &other, r"printf '\033]9;later\007'", "later");
    assert!(eventually(PATIENCE, || notified(&mut gui, &other)));
    assert!(
        !notified(&mut gui, &tab),
        "the replayed history notified again"
    );
}

#[test]
fn notifications_show_again_once_the_gui_is_gone() {
    let env = Env::new("notify-gone");
    let _daemon = env.start();
    let mut gui = env.connect("gui");
    let tab = new_tab(&mut gui, true);
    gui.call(&json!({"command": "activate", "tab": tab}));
    gui.call(&json!({"command": "set_window_open", "open": true}));
    drop(gui);

    let mut cli = env.connect("cli");
    assert!(eventually(PATIENCE, || {
        assert!(succeeded(
            &env.spawnshell(&["notify", "after", "--tab", &tab])
        ));
        notified(&mut cli, &tab)
    }));
}

#[test]
fn program_titles_reach_the_gui() {
    let env = Env::new("titles");
    let _daemon = env.start();
    let mut gui = env.connect("gui");
    let tab = new_tab(&mut gui, false);

    gui.type_text(&tab, concat!(r"printf '\033]2;building\007'", "\n"));
    let event = gui.event("title");
    assert_eq!(event["tab"], tab.as_str());
    assert_eq!(event["title"], "building");
}

fn previous_data_dir(env: &Env) -> PathBuf {
    env.home.join(DATA_ROOT).join("com.erblack.summon-terminal")
}

#[test]
fn data_of_the_previous_name_is_taken_over() {
    let env = Env::new("previous");
    let previous = previous_data_dir(&env);
    std::fs::create_dir_all(previous.join("history")).unwrap();
    std::fs::write(previous.join("session.json"), r#"{"tabs":[{"id":"kept"}]}"#).unwrap();
    std::fs::write(previous.join("history").join("kept"), "old output").unwrap();
    let _daemon = env.start();
    let mut gui = env.connect("gui");

    assert_eq!(gui.tabs()[0]["id"], "kept");
    gui.call(&json!({"command": "attach", "tab": "kept"}));
    gui.wait_for("old output");
    assert!(!previous.exists());
    assert!(env.data_dir().join("history").join("kept").exists());
}

#[test]
fn data_of_the_previous_name_waits_until_it_stops() {
    let env = Env::new("running");
    let previous = previous_data_dir(&env);
    std::fs::create_dir_all(&previous).unwrap();
    std::fs::write(previous.join("session.json"), r#"{"tabs":[]}"#).unwrap();
    let lock = Lock::open(&previous.join("daemon.lock")).unwrap();
    assert!(lock.try_lock().unwrap());
    let mut daemon = env.start();

    assert!(
        eventually(PATIENCE, || daemon.child.try_wait().unwrap().is_some()),
        "the daemon started next to the previous one"
    );
    assert!(!daemon.child.wait().unwrap().success());
    assert!(previous.join("session.json").exists());
    assert!(!env.data_dir().exists());
}

#[test]
fn the_daemon_says_its_version_and_pid() {
    let env = Env::new("version");
    let daemon = env.start();
    let mut cli = env.connect("cli");

    let hello = cli.call(&json!({"command": "hello", "role": "cli"}));

    assert_eq!(hello["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(hello["pid"], daemon.child.id());
}

#[test]
fn the_daemon_after_an_upgrade_reports_its_own_pid() {
    let env = Env::new("upgrade-pid");
    let mut first = env.start();
    let mut gui = env.connect("gui");
    let old = first.child.id();

    let _ = gui.try_call(&json!({"command": "upgrade", "exe": BIN}));
    assert!(first.exited(), "old daemon did not exit");

    let mut cli = env.connect("cli");
    let hello = cli.call(&json!({"command": "hello", "role": "cli"}));

    assert_ne!(hello["pid"], old);
    let pid: Pid = serde_json::from_value(hello["pid"].clone()).unwrap();
    assert!(process::exists(pid));
    assert_eq!(hello["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn version_in_a_terminal_shows_the_screen() {
    let env = Env::new("terminal-version");

    let output = Command::new("script")
        .args(["-q", "/dev/null", BIN, "--version"])
        .env("HOME", &env.home)
        .env("SPAWNSHELL_SOCKET", &env.socket)
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(succeeded(&output));
    let text = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    let title = format!("      spawnshell {}\n", env!("CARGO_PKG_VERSION"));
    assert!(text.contains(&title), "{text}");
    assert!(text.contains("daemon  not running\n"), "{text}");
}

#[test]
fn piped_version_is_one_line() {
    let env = Env::new("piped-version");

    for flag in ["--version", "-V"] {
        let output = env.spawnshell(&[flag]);

        assert!(succeeded(&output));
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("spawnshell {}\n", env!("CARGO_PKG_VERSION"))
        );
    }
}

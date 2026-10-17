//! The evdev side of the Linux listener: which input devices are read, which
//! keyboards are taken over so the key a hotkey ends in can be withheld from
//! the focused app, and how each is handed back.
//!
//! It replaces `rdev::grab`, which failed this listener three ways:
//!
//! * **A new input device ended capture for good.** rdev opens a node the
//!   moment inotify reports it, before udev has given it to the `input` group,
//!   so the open fails with `EACCES` and the whole grab returns. A keyboard
//!   reconnecting, a resume, or any program creating a uinput device did it,
//!   and every shortcut stayed dead until the app was restarted.
//! * **It left keys pressed in the compositor.** rdev took every device over
//!   at once, whatever was held. A key held across that instant (Super, when
//!   the app is opened or shown with Super+S) went down on the keyboard and
//!   came up on rdev's mirror of it, so the compositor kept it pressed.
//! * **It took over far more than it needed**: touchpads, mice, the lid switch
//!   and the power button, all re-sent through mirrors, when only the keys a
//!   hotkey ends in are ever withheld.
//!
//! Here each device is handled on its own:
//!
//! * A keyboard is taken over only while no key on it is held, checked again
//!   right after `EVIOCGRAB`; a key that went down in between makes it hand
//!   the keyboard back and wait for the release. Its mirror is created first
//!   and given [`Config::mirror_settle`] to be opened by the compositor, since
//!   anything sent through a mirror nobody has opened yet is lost.
//! * A key whose press was withheld has its repeats and release withheld too,
//!   and nothing else is (see `tracker.rs`).
//! * A mouse is taken over the same way, but only while one of its buttons is
//!   a shortcut (the middle button; left and right never are), so that click
//!   can be withheld from the app under the pointer. Otherwise it is only
//!   read, as are touchpads and anything another program created (keyd,
//!   ydotool, other listeners' mirrors).
//! * A node that refuses us is tried again when udev changes it (inotify
//!   `IN_ATTRIB`) and on a short schedule. A device that goes away is dropped
//!   alone; the others keep working.
//! * After `SYN_DROPPED`, libevdev replays the difference and it is forwarded
//!   like any other event, so a key whose release the kernel dropped cannot
//!   stay held on the mirror.

use std::collections::{HashMap, HashSet};
use std::ffi::{CString, OsStr};
use std::fs::{File, OpenOptions};
use std::io;
use std::mem::size_of;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::raw::c_int;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use evdev_rs::enums::{EventCode, EV_KEY};
use evdev_rs::{Device, GrabMode, InputEvent, ReadFlag, ReadStatus, UInputDevice};
use log::{debug, info, warn};

use super::hub::Hub;
use super::keycode;
use super::tracker::{
    open_retry_delay, role_for, takeover_retry_delay, DeviceId, Role, Tracker, Verdict, RELEASE,
    REPEAT,
};

/// Where input device nodes live.
pub(crate) const INPUT_DIR: &str = "/dev/input";
/// The sysfs directory of every device made through uinput: mirrors, ours
/// and everyone else's.
const UINPUT_SYSFS: &str = "/sys/devices/virtual/input";
/// How long the compositor gets to open a new mirror before its keyboard is
/// taken over. It needs tens of milliseconds.
const MIRROR_SETTLE: Duration = Duration::from_millis(500);
/// The kernel's bitmap of held keys, in bytes (`KEY_MAX` is 0x2ff).
const KEY_BITMAP_LEN: usize = 0x2ff / 8 + 1;
/// `EVIOCGKEY(len)`: `_IOR('E', 0x18, len)` in the generic ioctl layout
/// (x86, ARM, RISC-V).
const EVIOCGKEY: u64 = (2 << 30) | ((KEY_BITMAP_LEN as u64) << 16) | ((b'E' as u64) << 8) | 0x18;
const INOTIFY_TOKEN: u64 = u64::MAX;
const WAKE_TOKEN: u64 = u64::MAX - 1;

#[derive(Debug, Clone)]
pub(crate) struct Config {
    pub(crate) input_dir: PathBuf,
    /// Consider only devices whose name starts with this. Tests set it, so
    /// nothing but their own virtual keyboards is ever touched.
    pub(crate) only_named: Option<String>,
    /// Take over keyboards made through uinput too. Tests' keyboards are.
    pub(crate) grab_uinput: bool,
    pub(crate) mirror_settle: Duration,
}

impl Config {
    pub(crate) fn system() -> Self {
        Self {
            input_dir: PathBuf::from(INPUT_DIR),
            only_named: None,
            grab_uinput: false,
            mirror_settle: MIRROR_SETTLE,
        }
    }
}

enum Stage {
    /// Only read: a [`Role::Listen`] device, a device that cannot be mirrored,
    /// or a [`Role::Pointer`] no shortcut needs right now.
    Listening,
    /// A keyboard whose mirror is to be made at `at`.
    NeedsMirror { at: Instant },
    /// Mirror made; the compositor has until `until` to open it.
    Settling { until: Instant },
    /// To be taken over as soon as no key on it is held.
    Waiting,
    /// Another program holds it; try again at `until`.
    Refused { until: Instant },
    /// Taken over: everything it sends goes through the mirror.
    Grabbed,
}

struct Mirror {
    device: UInputDevice,
    syspath: PathBuf,
}

struct Tracked {
    path: PathBuf,
    name: String,
    device: Device,
    /// `device`'s descriptor, for the ioctls libevdev does not wrap. Valid as
    /// long as `device`.
    fd: RawFd,
    stage: Stage,
    mirror: Option<Mirror>,
    /// Failed mirror or takeover attempts in a row.
    attempts: u32,
    /// Whether "waiting for keys to be released" was logged for this wait.
    told_held: bool,
    /// A [`Role::Pointer`]: taken over only while a shortcut uses its buttons.
    pointer: bool,
    /// False once making its mirror failed for good (no `/dev/uinput`).
    can_mirror: bool,
}

struct OpenRetry {
    attempts: u32,
    /// `None`: stopped retrying until the node changes.
    at: Option<Instant>,
}

pub(crate) struct Engine {
    config: Config,
    hub: Arc<Hub>,
    epoll: OwnedFd,
    inotify: OwnedFd,
    next_id: DeviceId,
    devices: HashMap<DeviceId, Tracked>,
    by_path: HashMap<PathBuf, DeviceId>,
    /// Nodes that could not be opened yet.
    retries: HashMap<PathBuf, OpenRetry>,
    /// Nodes of no interest, until they are removed.
    ignored: HashSet<PathBuf>,
    /// sysfs paths of our own mirrors, so they are never read back.
    mirrors: HashSet<PathBuf>,
    tracker: Tracker,
    denied: bool,
    told_access: bool,
    told_uinput: bool,
    told_active: bool,
}

impl Engine {
    pub(crate) fn new(config: Config, hub: Arc<Hub>) -> io::Result<Self> {
        let epoll = owned(unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) })?;
        let inotify = watch_dir(&config.input_dir)?;
        epoll_add(&epoll, inotify.as_raw_fd(), INOTIFY_TOKEN)?;
        epoll_add(&epoll, hub.wake_fd(), WAKE_TOKEN)?;
        Ok(Self {
            config,
            hub,
            epoll,
            inotify,
            next_id: 0,
            devices: HashMap::new(),
            by_path: HashMap::new(),
            retries: HashMap::new(),
            ignored: HashSet::new(),
            mirrors: HashSet::new(),
            tracker: Tracker::default(),
            denied: false,
            told_access: false,
            told_uinput: false,
            told_active: false,
        })
    }

    /// Read and take over input devices until no listener is left. Every
    /// device is handed back when the engine is dropped.
    pub(crate) fn run(&mut self) -> io::Result<()> {
        self.scan(Instant::now());
        self.sync_pointers(Instant::now());
        if self.denied && self.devices.is_empty() && !self.told_access {
            self.told_access = true;
            warn!(
                "handy-keys: permission denied reading {}; global shortcuts will not work \
                 until this user can read the input devices. Fix: `sudo usermod -aG input \
                 $USER`, then log out and back in, or switch the keyboard engine to Tauri \
                 in Settings.",
                self.config.input_dir.display()
            );
        }

        let mut ready = [libc::epoll_event { events: 0, u64: 0 }; 16];
        while self.hub.has_live_subscribers() {
            let timeout = self.timeout(Instant::now());
            let count = epoll_wait(&self.epoll, &mut ready, timeout)?;
            for event in &ready[..count] {
                let (token, flags) = (event.u64, event.events);
                match token {
                    INOTIFY_TOKEN => self.read_inotify(),
                    // A listener came or went, or a mouse button stopped or
                    // started being a shortcut.
                    WAKE_TOKEN => {
                        self.hub.clear_wake();
                        self.sync_pointers(Instant::now());
                    }
                    id => self.device_ready(id, flags),
                }
            }
            self.run_due(Instant::now());
        }
        Ok(())
    }

    fn scan(&mut self, now: Instant) {
        let entries = match std::fs::read_dir(&self.config.input_dir) {
            Ok(entries) => entries,
            Err(error) => {
                warn!(
                    "handy-keys: cannot list {}: {error}",
                    self.config.input_dir.display()
                );
                return;
            }
        };
        let mut nodes: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| is_event_node(path))
            .collect();
        nodes.sort();
        for node in nodes {
            self.consider(node, now);
        }
    }

    /// Open a node and decide what it is for, unless it is known already.
    fn consider(&mut self, path: PathBuf, now: Instant) {
        if self.by_path.contains_key(&path) || self.ignored.contains(&path) {
            return;
        }
        let sysfs = sysfs_dir(&path);
        if sysfs.as_ref().is_some_and(|dir| self.mirrors.contains(dir)) {
            self.retries.remove(&path);
            self.ignored.insert(path);
            return;
        }
        let file = match open_node(&path) {
            Ok(file) => file,
            Err(error) if is_denied(&error) => {
                self.denied = true;
                self.retry_open_later(path, now);
                return;
            }
            // Gone already, or not an input device after all.
            Err(_) => {
                self.retries.remove(&path);
                return;
            }
        };
        self.retries.remove(&path);
        let fd = file.as_raw_fd();
        let device = match Device::new_from_fd(file) {
            Ok(device) => device,
            Err(error) => {
                debug!("handy-keys: skipping {} ({error})", path.display());
                self.ignored.insert(path);
                return;
            }
        };
        let name = device.name().unwrap_or_default().to_owned();
        if let Some(prefix) = &self.config.only_named {
            if !name.starts_with(prefix.as_str()) {
                self.ignored.insert(path);
                return;
            }
        }
        let made_by_uinput = !self.config.grab_uinput
            && sysfs
                .as_deref()
                .is_some_and(|dir| dir.starts_with(UINPUT_SYSFS));
        let has = |code: &EV_KEY| device.has(&EventCode::EV_KEY(code.clone()));
        let role = role_for(
            keycode::has_hotkey_key(has),
            keycode::has_bindable_button(has),
            keycode::has_followed_key(has),
            made_by_uinput,
        );
        let Some(role) = role else {
            self.ignored.insert(path);
            return;
        };

        let id = self.next_id;
        self.next_id += 1;
        if let Err(error) = epoll_add(&self.epoll, fd, id) {
            warn!("handy-keys: cannot watch '{name}' ({error})");
            return;
        }
        debug!(
            "handy-keys: reading '{name}' ({}){}",
            path.display(),
            match role {
                Role::Grab => " to take it over",
                Role::Pointer => ", to take over while a shortcut uses its buttons",
                Role::Listen => "",
            }
        );
        let stage = match role {
            Role::Grab => Stage::NeedsMirror { at: now },
            Role::Pointer | Role::Listen => Stage::Listening,
        };
        self.by_path.insert(path.clone(), id);
        self.devices.insert(
            id,
            Tracked {
                path,
                name,
                device,
                fd,
                stage,
                mirror: None,
                attempts: 0,
                told_held: false,
                pointer: role == Role::Pointer,
                can_mirror: true,
            },
        );
        match role {
            Role::Grab => self.make_mirror(id, now),
            Role::Pointer if self.hub.withholds_buttons() => self.make_mirror(id, now),
            Role::Pointer | Role::Listen => {}
        }
    }

    /// Take over the mice with a button a shortcut can use while some shortcut
    /// uses one, and hand them back once none does. A mouse otherwise goes to
    /// the compositor directly, with no mirror in between.
    fn sync_pointers(&mut self, now: Instant) {
        let wanted = self.hub.withholds_buttons();
        let pointers: Vec<DeviceId> = self
            .devices
            .iter()
            .filter(|(_, tracked)| tracked.pointer)
            .map(|(&id, _)| id)
            .collect();
        for id in pointers {
            let Some(tracked) = self.devices.get(&id) else {
                continue;
            };
            let idle = matches!(tracked.stage, Stage::Listening) && tracked.mirror.is_none();
            if wanted && idle && tracked.can_mirror {
                self.make_mirror(id, now);
            } else if !wanted && !idle {
                self.hand_back(id);
            }
        }
    }

    /// Give a device back to the compositor and only read it from now on.
    fn hand_back(&mut self, id: DeviceId) {
        let Some(tracked) = self.devices.get_mut(&id) else {
            return;
        };
        // The device first, then its mirror, as in `drop_device`.
        if matches!(tracked.stage, Stage::Grabbed) {
            let _ = tracked.device.grab(GrabMode::Ungrab);
        }
        if let Some(mirror) = tracked.mirror.take() {
            self.mirrors.remove(&mirror.syspath);
        }
        tracked.stage = Stage::Listening;
        tracked.attempts = 0;
        tracked.told_held = false;
        debug!(
            "handy-keys: handed '{}' back; no shortcut uses its buttons",
            tracked.name
        );
        self.tracker.forget_withheld(id);
    }

    fn retry_open_later(&mut self, path: PathBuf, now: Instant) {
        let retry = self.retries.entry(path).or_insert(OpenRetry {
            attempts: 0,
            at: None,
        });
        retry.attempts += 1;
        retry.at = open_retry_delay(retry.attempts).map(|delay| now + delay);
    }

    fn make_mirror(&mut self, id: DeviceId, now: Instant) {
        let Some(tracked) = self.devices.get_mut(&id) else {
            return;
        };
        match UInputDevice::create_from_device(&tracked.device) {
            Ok(device) => match device.syspath().map(canonical) {
                Some(syspath) => {
                    self.mirrors.insert(syspath.clone());
                    tracked.mirror = Some(Mirror { device, syspath });
                    tracked.stage = Stage::Settling {
                        until: now + self.config.mirror_settle,
                    };
                }
                None => {
                    // Without its sysfs path the mirror would be read back as
                    // one more keyboard, every key arriving twice.
                    debug!(
                        "handy-keys: no sysfs path for the mirror of '{}'; only reading it",
                        tracked.name
                    );
                    tracked.stage = Stage::Listening;
                    tracked.can_mirror = false;
                }
            },
            Err(error) if is_denied(&error) || error.kind() == io::ErrorKind::NotFound => {
                tracked.stage = Stage::Listening;
                tracked.can_mirror = false;
                if !self.told_uinput {
                    self.told_uinput = true;
                    warn!(
                        "handy-keys: cannot create a uinput device ({error}). Shortcuts still \
                         work, but the key that ends one also reaches the focused app. Making \
                         /dev/uinput writable for the 'input' group fixes it: KERNEL==\"uinput\", \
                         GROUP=\"input\", MODE=\"0660\" in a udev rule."
                    );
                }
            }
            Err(error) => {
                tracked.attempts += 1;
                let delay = takeover_retry_delay(tracked.attempts);
                debug!(
                    "handy-keys: could not mirror '{}' ({error}); trying again in {delay:?}",
                    tracked.name
                );
                tracked.stage = Stage::NeedsMirror { at: now + delay };
            }
        }
    }

    /// Take a waiting keyboard over if no key on it is held.
    fn try_take_over(&mut self, id: DeviceId) {
        if !matches!(
            self.devices.get(&id).map(|tracked| &tracked.stage),
            Some(Stage::Waiting)
        ) {
            return;
        }
        // Whatever is queued reached the compositor already: read it as a
        // listener, never forward it.
        if !self.pump(id) {
            return;
        }
        let Some(tracked) = self.devices.get_mut(&id) else {
            return;
        };
        match any_key_held(tracked.fd) {
            Ok(false) => {}
            Ok(true) => {
                if !tracked.told_held {
                    tracked.told_held = true;
                    debug!(
                        "handy-keys: waiting for the keys held on '{}' to be released before taking it over",
                        tracked.name
                    );
                }
                return;
            }
            Err(error) => {
                debug!(
                    "handy-keys: cannot read the keys held on '{}' ({error})",
                    tracked.name
                );
                return;
            }
        }
        if let Err(error) = tracked.device.grab(GrabMode::Grab) {
            tracked.attempts += 1;
            let delay = takeover_retry_delay(tracked.attempts);
            debug!(
                "handy-keys: cannot take over '{}' ({error}), another program may hold it; trying again in {delay:?}",
                tracked.name
            );
            tracked.stage = Stage::Refused {
                until: Instant::now() + delay,
            };
            return;
        }
        // A key that went down in the last instant was seen by the compositor,
        // and its release would now come here instead. Hand the keyboard back
        // and wait for that release.
        if !matches!(any_key_held(tracked.fd), Ok(false)) {
            let _ = tracked.device.grab(GrabMode::Ungrab);
            return;
        }
        tracked.stage = Stage::Grabbed;
        tracked.attempts = 0;
        tracked.told_held = false;
        debug!(
            "handy-keys: took over '{}' ({})",
            tracked.name,
            tracked.path.display()
        );
        self.tracker.forget_withheld(id);
        if !self.told_active {
            self.told_active = true;
            info!("handy-keys: keyboard capture active");
        }
    }

    /// Read and handle everything the device has queued. `false` if it is gone.
    fn pump(&mut self, id: DeviceId) -> bool {
        let mut batch = Vec::new();
        let gone = match self.devices.get(&id) {
            Some(tracked) => read_events(&tracked.device, &mut batch),
            None => return false,
        };
        for event in &batch {
            self.handle(id, event);
        }
        if gone {
            self.drop_device(id, true);
            return false;
        }
        true
    }

    fn handle(&mut self, id: DeviceId, event: &InputEvent) {
        let grabbed = matches!(
            self.devices.get(&id).map(|tracked| &tracked.stage),
            Some(Stage::Grabbed)
        );
        let verdict = match &event.event_code {
            EventCode::EV_KEY(code) => self.key(id, code, event.value, grabbed),
            _ => Verdict::Forward,
        };
        if grabbed && verdict == Verdict::Forward {
            self.forward(id, event);
        }
    }

    fn key(&mut self, id: DeviceId, code: &EV_KEY, value: i32, grabbed: bool) -> Verdict {
        if let Some(modifier) = keycode::modifier_for(code) {
            if value != REPEAT {
                if let Some(event) = self.tracker.modifier(id, modifier, value != RELEASE) {
                    self.hub.broadcast(event);
                }
            }
            // Never withheld: a withheld modifier whose release is then lost
            // (a crash, a hang) would stay pressed in the compositor.
            return Verdict::Forward;
        }
        if let Some(key) = keycode::key_for(code) {
            let hub = &self.hub;
            let (verdict, event) =
                self.tracker
                    .key(id, keycode::code_of(code), key, value, |modifiers, key| {
                        grabbed && hub.should_block(modifiers, key)
                    });
            self.hub.broadcast(event);
            return verdict;
        }
        if let Some(button) = keycode::button_for(code) {
            let hub = &self.hub;
            let (verdict, event) = self.tracker.button(
                id,
                keycode::code_of(code),
                button,
                value,
                |modifiers, key| grabbed && hub.should_block(modifiers, key),
            );
            if let Some(event) = event {
                self.hub.broadcast(event);
            }
            return verdict;
        }
        Verdict::Forward
    }

    fn forward(&mut self, id: DeviceId, event: &InputEvent) {
        let Some(tracked) = self.devices.get_mut(&id) else {
            return;
        };
        let Some(mirror) = &tracked.mirror else {
            return;
        };
        let Err(error) = mirror.device.write_event(event) else {
            return;
        };
        warn!(
            "handy-keys: could not pass a key on from '{}' ({error}); handing the keyboard back",
            tracked.name
        );
        let _ = tracked.device.grab(GrabMode::Ungrab);
        if let Some(mirror) = tracked.mirror.take() {
            self.mirrors.remove(&mirror.syspath);
        }
        tracked.attempts += 1;
        tracked.stage = Stage::NeedsMirror {
            at: Instant::now() + takeover_retry_delay(tracked.attempts),
        };
        self.tracker.forget_withheld(id);
    }

    fn device_ready(&mut self, id: DeviceId, flags: u32) {
        if !self.pump(id) {
            return;
        }
        if flags & (libc::EPOLLHUP | libc::EPOLLERR) as u32 != 0 {
            self.drop_device(id, true);
            return;
        }
        self.try_take_over(id);
    }

    /// Stop reading a device. `gone`: it disappeared or failed, rather than
    /// being let go.
    fn drop_device(&mut self, id: DeviceId, gone: bool) {
        let Some(mut tracked) = self.devices.remove(&id) else {
            return;
        };
        self.by_path.remove(&tracked.path);
        let _ = epoll_del(&self.epoll, tracked.fd);
        if matches!(tracked.stage, Stage::Grabbed) {
            let _ = tracked.device.grab(GrabMode::Ungrab);
        }
        // The keyboard first, then its mirror: what is held on the mirror is
        // released by the compositor when it disappears, and the keyboard
        // reaches the compositor directly from then on.
        if let Some(mirror) = tracked.mirror.take() {
            self.mirrors.remove(&mirror.syspath);
            drop(mirror);
        }
        debug!(
            "handy-keys: {} '{}'",
            if gone { "lost" } else { "let go of" },
            tracked.name
        );
        let path = tracked.path.clone();
        drop(tracked);
        if let Some(event) = self.tracker.remove(id) {
            self.hub.broadcast(event);
        }
        if gone {
            // A read error on a node that is still there: open it again later.
            // If it was unplugged the open finds nothing and that is the end.
            self.retry_open_later(path, Instant::now());
        }
    }

    fn node_removed(&mut self, path: &Path) {
        if let Some(&id) = self.by_path.get(path) {
            self.drop_device(id, true);
        }
        self.retries.remove(path);
        self.ignored.remove(path);
    }

    fn read_inotify(&mut self) {
        const HEADER: usize = size_of::<libc::inotify_event>();
        let mut buffer = [0u8; 4096];
        let mut rescan = false;
        let now = Instant::now();
        loop {
            // SAFETY: reads at most `buffer.len()` bytes into `buffer`.
            let read = unsafe {
                libc::read(
                    self.inotify.as_raw_fd(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                )
            };
            if read <= 0 {
                break;
            }
            let read = read as usize;
            let mut offset = 0;
            while offset + HEADER <= read {
                // SAFETY: a whole header is inside the bytes read.
                let header: libc::inotify_event =
                    unsafe { std::ptr::read_unaligned(buffer.as_ptr().add(offset).cast()) };
                let name_start = offset + HEADER;
                let name_end = (name_start + header.len as usize).min(read);
                offset = name_end;
                if header.mask & libc::IN_Q_OVERFLOW != 0 {
                    rescan = true;
                    continue;
                }
                let name = &buffer[name_start..name_end];
                let name = &name[..name
                    .iter()
                    .position(|&byte| byte == 0)
                    .unwrap_or(name.len())];
                let path = self.config.input_dir.join(OsStr::from_bytes(name));
                if !is_event_node(&path) {
                    continue;
                }
                if header.mask & (libc::IN_DELETE | libc::IN_MOVED_FROM) != 0 {
                    self.node_removed(&path);
                } else {
                    // Created, or udev changed its owner or mode: try it now.
                    self.consider(path, now);
                }
            }
        }
        // Events were lost, possibly a removal: a node set aside as of no
        // interest may hold a keyboard now. Look at everything again.
        if rescan {
            self.ignored.clear();
            self.scan(now);
        }
    }

    fn run_due(&mut self, now: Instant) {
        let due: Vec<PathBuf> = self
            .retries
            .iter()
            .filter(|(_, retry)| retry.at.is_some_and(|at| at <= now))
            .map(|(path, _)| path.clone())
            .collect();
        for path in due {
            self.consider(path, now);
        }

        let due: Vec<(DeviceId, bool)> = self
            .devices
            .iter()
            .filter_map(|(&id, tracked)| match tracked.stage {
                Stage::NeedsMirror { at } if at <= now => Some((id, true)),
                Stage::Settling { until } | Stage::Refused { until } if until <= now => {
                    Some((id, false))
                }
                _ => None,
            })
            .collect();
        for (id, needs_mirror) in due {
            if needs_mirror {
                self.make_mirror(id, now);
                continue;
            }
            if let Some(tracked) = self.devices.get_mut(&id) {
                tracked.stage = Stage::Waiting;
            }
            self.try_take_over(id);
        }
    }

    /// Milliseconds until the next timed step, or -1 for none.
    fn timeout(&self, now: Instant) -> i32 {
        let retries = self.retries.values().filter_map(|retry| retry.at);
        let stages = self
            .devices
            .values()
            .filter_map(|tracked| match tracked.stage {
                Stage::NeedsMirror { at } => Some(at),
                Stage::Settling { until } | Stage::Refused { until } => Some(until),
                _ => None,
            });
        match retries.chain(stages).min() {
            None => -1,
            // Rounded up, so the loop never wakes just short of a deadline.
            Some(at) => {
                let wait = at
                    .saturating_duration_since(now)
                    .as_millis()
                    .saturating_add(1);
                i32::try_from(wait).unwrap_or(i32::MAX)
            }
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let ids: Vec<DeviceId> = self.devices.keys().copied().collect();
        for id in ids {
            self.drop_device(id, false);
        }
    }
}

/// Read what the device has queued into `batch`. `true` if it is gone.
///
/// Reading only stops at `EAGAIN`, never at a count: libevdev keeps a queue of
/// its own, and events left in it wake nothing once the descriptor is drained,
/// so they would wait for the device's next event.
fn read_events(device: &Device, batch: &mut Vec<InputEvent>) -> bool {
    loop {
        match device.next_event(ReadFlag::NORMAL) {
            Ok((ReadStatus::Success, event)) => batch.push(event),
            // SYN_DROPPED: the kernel's queue overflowed. libevdev replays the
            // difference between what was read and the device's state now.
            Ok((ReadStatus::Sync, _)) => loop {
                match device.next_event(ReadFlag::SYNC) {
                    Ok((_, event)) => batch.push(event),
                    Err(error) if would_block(&error) || interrupted(&error) => break,
                    Err(_) => return true,
                }
            },
            Err(error) if would_block(&error) || interrupted(&error) => return false,
            Err(_) => return true,
        }
    }
}

fn is_event_node(path: &Path) -> bool {
    path.file_name()
        .and_then(OsStr::to_str)
        .and_then(|name| name.strip_prefix("event"))
        .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
}

/// The sysfs directory of the input device behind a node.
fn sysfs_dir(node: &Path) -> Option<PathBuf> {
    let name = node.file_name()?;
    std::fs::canonicalize(Path::new("/sys/class/input").join(name).join("device")).ok()
}

fn canonical(path: &str) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path))
}

fn open_node(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
}

fn is_denied(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(libc::EACCES | libc::EPERM))
}

fn would_block(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::EAGAIN)
}

fn interrupted(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::EINTR)
}

/// Whether the kernel reports any key or button on the device as held now.
fn any_key_held(fd: RawFd) -> io::Result<bool> {
    let mut bits = [0u8; KEY_BITMAP_LEN];
    // SAFETY: EVIOCGKEY writes at most `KEY_BITMAP_LEN` bytes into `bits`.
    let result = unsafe { libc::ioctl(fd, EVIOCGKEY as _, bits.as_mut_ptr()) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(bits.iter().any(|&byte| byte != 0))
}

fn owned(fd: c_int) -> io::Result<OwnedFd> {
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: `fd` is a new descriptor nothing else owns.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

fn check(result: c_int) -> io::Result<()> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn watch_dir(dir: &Path) -> io::Result<OwnedFd> {
    let fd = owned(unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) })?;
    let path = CString::new(dir.as_os_str().as_bytes())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mask = libc::IN_CREATE
        | libc::IN_ATTRIB
        | libc::IN_DELETE
        | libc::IN_MOVED_TO
        | libc::IN_MOVED_FROM;
    // SAFETY: `path` is a valid C string for the duration of the call.
    check(unsafe { libc::inotify_add_watch(fd.as_raw_fd(), path.as_ptr(), mask) })?;
    Ok(fd)
}

fn epoll_add(epoll: &OwnedFd, fd: RawFd, token: u64) -> io::Result<()> {
    let mut event = libc::epoll_event {
        events: libc::EPOLLIN as u32,
        u64: token,
    };
    // SAFETY: `event` outlives the call.
    check(unsafe { libc::epoll_ctl(epoll.as_raw_fd(), libc::EPOLL_CTL_ADD, fd, &mut event) })
}

fn epoll_del(epoll: &OwnedFd, fd: RawFd) -> io::Result<()> {
    let mut event = libc::epoll_event { events: 0, u64: 0 };
    // SAFETY: `event` outlives the call.
    check(unsafe { libc::epoll_ctl(epoll.as_raw_fd(), libc::EPOLL_CTL_DEL, fd, &mut event) })
}

fn epoll_wait(epoll: &OwnedFd, ready: &mut [libc::epoll_event], timeout: i32) -> io::Result<usize> {
    // SAFETY: the kernel writes at most `ready.len()` entries.
    let count = unsafe {
        libc::epoll_wait(
            epoll.as_raw_fd(),
            ready.as_mut_ptr(),
            ready.len() as c_int,
            timeout,
        )
    };
    if count < 0 {
        let error = io::Error::last_os_error();
        return if error.kind() == io::ErrorKind::Interrupted {
            Ok(0)
        } else {
            Err(error)
        };
    }
    Ok(count as usize)
}

#[cfg(test)]
mod tests {
    //! Live checks against virtual keyboards. They need the `input` group and
    //! write access to `/dev/uinput`, and the desktop sees their keyboards
    //! for a few seconds (only a Right Ctrl tap ever reaches it), so they are
    //! ignored by default:
    //! `cargo test -p handy-keys --lib -- --ignored --nocapture --test-threads=1`.
    //! The engines they start consider only their own keyboards, never the
    //! machine's.

    use super::super::hub::Subscriber;
    use super::*;
    use crate::platform::state::BlockingHotkeys;
    use crate::types::{Hotkey, Key, KeyEvent, Modifiers};
    use evdev_rs::enums::{EV_REL, EV_SYN};
    use evdev_rs::TimeVal;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Receiver};
    use std::sync::Mutex;
    use std::thread::{self, JoinHandle};

    struct Listener {
        hub: Arc<Hub>,
        events: Receiver<KeyEvent>,
        running: Arc<AtomicBool>,
        engine: Option<JoinHandle<()>>,
    }

    impl Listener {
        fn start(prefix: &str, blocking: Option<BlockingHotkeys>) -> Self {
            let hub = Arc::new(Hub::new().expect("eventfd"));
            let (sender, events) = mpsc::channel();
            let running = Arc::new(AtomicBool::new(true));
            hub.subscribe(Subscriber {
                sender,
                running: Arc::clone(&running),
                blocking,
            });
            let config = Config {
                input_dir: PathBuf::from(INPUT_DIR),
                only_named: Some(prefix.to_owned()),
                grab_uinput: true,
                mirror_settle: Duration::from_millis(150),
            };
            let engine_hub = Arc::clone(&hub);
            let engine = thread::spawn(move || {
                Engine::new(config, engine_hub)
                    .expect("engine")
                    .run()
                    .expect("engine run");
            });
            Self {
                hub,
                events,
                running,
                engine: Some(engine),
            }
        }

        /// The last listener leaving ends the engine, which hands back every
        /// device it took.
        fn stop(mut self) {
            self.running.store(false, Ordering::SeqCst);
            self.hub.wake();
            self.engine.take().unwrap().join().expect("engine thread");
        }

        fn received(&self) -> Vec<KeyEvent> {
            thread::sleep(Duration::from_millis(250));
            self.events.try_iter().collect()
        }
    }

    fn probe(name: &str) -> UInputDevice {
        let template = Device::new().expect("libevdev");
        template.set_name(name);
        // Right Ctrl and F12 for a hotkey; BTN_TRIGGER_HAPPY1 stands in for a
        // key held at startup, because libinput drops it on a keyboard and so
        // holding it reaches no app.
        for code in [
            EV_KEY::KEY_RIGHTCTRL,
            EV_KEY::KEY_F12,
            EV_KEY::BTN_TRIGGER_HAPPY1,
        ] {
            template
                .enable(&EventCode::EV_KEY(code))
                .expect("enable key");
        }
        template
            .enable(&EventCode::EV_SYN(EV_SYN::SYN_REPORT))
            .expect("enable syn");
        let probe = UInputDevice::create_from_device(&template)
            .expect("create a uinput keyboard (needs write access to /dev/uinput)");
        let node = node_of(&probe);
        wait_until(Duration::from_secs(3), || File::open(&node).is_ok());
        probe
    }

    fn node_of(probe: &UInputDevice) -> String {
        probe.devnode().expect("probe devnode").to_owned()
    }

    fn send(probe: &UInputDevice, code: EV_KEY, value: i32) {
        let time = TimeVal::new(0, 0);
        probe
            .write_event(&InputEvent::new(&time, &EventCode::EV_KEY(code), value))
            .expect("write key");
        probe
            .write_event(&InputEvent::new(
                &time,
                &EventCode::EV_SYN(EV_SYN::SYN_REPORT),
                0,
            ))
            .expect("write syn");
    }

    /// Whether some program holds the node grabbed: our own grab then fails.
    fn is_grabbed(node: &str) -> bool {
        let mut device =
            Device::new_from_fd(File::open(node).expect("open node")).expect("libevdev");
        match device.grab(GrabMode::Grab) {
            Ok(()) => {
                let _ = device.grab(GrabMode::Ungrab);
                false
            }
            Err(error) => {
                assert_eq!(error.raw_os_error(), Some(libc::EBUSY), "{error}");
                true
            }
        }
    }

    /// The engine's mirror of a probe: the other node with the probe's name.
    fn mirror_of(name: &str, probe_node: &str) -> Device {
        let mut found = None;
        wait_until(Duration::from_secs(3), || {
            found = std::fs::read_dir("/sys/class/input")
                .ok()
                .and_then(|entries| {
                    entries.flatten().find_map(|entry| {
                        let node = format!("{INPUT_DIR}/{}", entry.file_name().to_string_lossy());
                        let same_name = std::fs::read_to_string(entry.path().join("device/name"))
                            .is_ok_and(|n| n.trim_end() == name);
                        (same_name && node != probe_node && is_event_node(Path::new(&node)))
                            .then(|| open_node(Path::new(&node)).ok())
                            .flatten()
                    })
                });
            found.is_some()
        });
        Device::new_from_fd(found.expect("the probe's mirror")).expect("libevdev")
    }

    fn keys_sent_through(mirror: &Device) -> Vec<(u16, i32)> {
        let mut batch = Vec::new();
        read_events(mirror, &mut batch);
        batch
            .iter()
            .filter_map(|event| match &event.event_code {
                EventCode::EV_KEY(code) => Some((keycode::code_of(code), event.value)),
                _ => None,
            })
            .collect()
    }

    fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + limit;
        while !done() {
            assert!(Instant::now() < deadline, "timed out");
            thread::sleep(Duration::from_millis(25));
        }
    }

    const RIGHT_CTRL: u16 = 97;

    #[test]
    #[ignore = "needs the input group and /dev/uinput; see the module docs"]
    fn a_keyboard_is_taken_over_only_with_no_key_held_and_withholds_only_the_hotkey_key() {
        let name = format!("hk-probe-takeover-{}", std::process::id());
        let probe = probe(&name);
        let node = node_of(&probe);
        send(&probe, EV_KEY::BTN_TRIGGER_HAPPY1, 1);

        let hotkey = Hotkey::new(Modifiers::CTRL_RIGHT, Key::F12).unwrap();
        let blocking: BlockingHotkeys = Arc::new(Mutex::new(HashSet::from([hotkey])));
        let listener = Listener::start(&name, Some(blocking));
        thread::sleep(Duration::from_millis(900));
        assert!(!is_grabbed(&node), "taken over while a key was held");

        send(&probe, EV_KEY::BTN_TRIGGER_HAPPY1, 0);
        thread::sleep(Duration::from_millis(300));
        assert!(is_grabbed(&node), "not taken over once the key came up");

        let mirror = mirror_of(&name, &node);
        keys_sent_through(&mirror);
        send(&probe, EV_KEY::KEY_RIGHTCTRL, 1);
        send(&probe, EV_KEY::KEY_F12, 1);
        send(&probe, EV_KEY::KEY_F12, 0);
        send(&probe, EV_KEY::KEY_RIGHTCTRL, 0);

        let received = listener.received();
        let summary: Vec<_> = received
            .iter()
            .map(|event| (event.modifiers, event.key, event.is_key_down))
            .collect();
        assert_eq!(
            summary,
            [
                (Modifiers::CTRL_RIGHT, None, true),
                (Modifiers::CTRL_RIGHT, Some(Key::F12), true),
                (Modifiers::CTRL_RIGHT, Some(Key::F12), false),
                (Modifiers::empty(), None, false),
            ],
            "the listener hears every key"
        );
        assert_eq!(
            keys_sent_through(&mirror),
            [(RIGHT_CTRL, 1), (RIGHT_CTRL, 0)],
            "the desktop gets Ctrl but not the hotkey's F12"
        );

        listener.stop();
        assert!(
            !is_grabbed(&node),
            "still held after the last listener left"
        );
    }

    #[test]
    #[ignore = "needs the input group and /dev/uinput; see the module docs"]
    fn a_mouse_is_taken_over_only_while_its_middle_button_is_a_shortcut() {
        let name = format!("hk-probe-pointer-{}", std::process::id());
        let template = Device::new().expect("libevdev");
        template.set_name(&name);
        for code in [EV_KEY::BTN_LEFT, EV_KEY::BTN_MIDDLE] {
            template
                .enable(&EventCode::EV_KEY(code))
                .expect("enable button");
        }
        for code in [EV_REL::REL_X, EV_REL::REL_Y] {
            template
                .enable(&EventCode::EV_REL(code))
                .expect("enable axis");
        }
        template
            .enable(&EventCode::EV_SYN(EV_SYN::SYN_REPORT))
            .expect("enable syn");
        let mouse = UInputDevice::create_from_device(&template)
            .expect("create a uinput mouse (needs write access to /dev/uinput)");
        let node = node_of(&mouse);
        wait_until(Duration::from_secs(3), || File::open(&node).is_ok());

        let blocking: BlockingHotkeys = Arc::new(Mutex::new(HashSet::new()));
        let listener = Listener::start(&name, Some(Arc::clone(&blocking)));
        thread::sleep(Duration::from_millis(600));
        assert!(!is_grabbed(&node), "a mouse no shortcut needs is only read");

        let middle = Hotkey::new(Modifiers::empty(), Key::MouseMiddle).unwrap();
        blocking.lock().unwrap().insert(middle);
        listener.hub.wake();
        thread::sleep(Duration::from_millis(600));
        assert!(
            is_grabbed(&node),
            "not taken over once its middle button became a shortcut"
        );

        let mirror = mirror_of(&name, &node);
        keys_sent_through(&mirror);
        listener.received();
        send(&mouse, EV_KEY::BTN_MIDDLE, 1);
        send(&mouse, EV_KEY::BTN_MIDDLE, 0);
        let heard: Vec<_> = listener
            .received()
            .iter()
            .map(|event| (event.key, event.is_key_down))
            .collect();
        assert_eq!(
            heard,
            [
                (Some(Key::MouseMiddle), true),
                (Some(Key::MouseMiddle), false)
            ],
            "the listener hears the click"
        );
        assert!(
            keys_sent_through(&mirror).is_empty(),
            "the desktop got the click a shortcut used"
        );

        blocking.lock().unwrap().clear();
        listener.hub.wake();
        thread::sleep(Duration::from_millis(300));
        assert!(
            !is_grabbed(&node),
            "still held once no shortcut used its buttons"
        );
        listener.stop();
    }

    #[test]
    #[ignore = "needs the input group and /dev/uinput; see the module docs"]
    fn a_keyboard_coming_and_going_leaves_the_others_working() {
        let name = format!("hk-probe-hotplug-{}", std::process::id());
        let first = probe(&format!("{name} first"));
        let first_node = node_of(&first);
        let listener = Listener::start(&name, Some(Arc::new(Mutex::new(HashSet::new()))));
        thread::sleep(Duration::from_millis(600));
        assert!(is_grabbed(&first_node));

        // Plugged in while the engine runs: rdev's grab ended for good here.
        let second = probe(&format!("{name} second"));
        let second_node = node_of(&second);
        thread::sleep(Duration::from_millis(1_200));
        assert!(
            is_grabbed(&second_node),
            "a keyboard plugged in later was not taken over"
        );
        listener.received();
        send(&second, EV_KEY::KEY_RIGHTCTRL, 1);
        send(&second, EV_KEY::KEY_RIGHTCTRL, 0);
        assert_eq!(
            listener.received().len(),
            2,
            "no keys from the new keyboard"
        );

        drop(second);
        thread::sleep(Duration::from_millis(300));
        send(&first, EV_KEY::KEY_RIGHTCTRL, 1);
        send(&first, EV_KEY::KEY_RIGHTCTRL, 0);
        assert_eq!(
            listener.received().len(),
            2,
            "capture ended when a keyboard went away"
        );
        assert!(is_grabbed(&first_node));

        listener.stop();
        assert!(!is_grabbed(&first_node));
    }
}

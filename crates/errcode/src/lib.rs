//! Stable codes for every failure rhythr can put in front of a user.
//!
//! A bug report reads "it did not work". The message underneath it is a
//! sentence written for the person in front of the app, which is the right
//! thing to show and the wrong thing to search for: it gets translated by the
//! reporter, truncated by the screenshot, or paraphrased from memory. A code
//! is none of those things. It is short enough to type, stable across
//! releases, and it maps to one row in `docs/ERROR-CODES.md` that says what
//! actually happened.
//!
//! Codes look like `RH-FFM-101-L`:
//!
//! * `FFM` is the area (see [`Area`]),
//! * `101` is the failure inside that area,
//! * `L` is the operating system the code was produced on (`W`indows,
//!   `L`inux, `M`acOS).
//!
//! The platform suffix is there because the same failure has a different
//! cause on each system: "ffmpeg not found" means "not on PATH and not next
//! to the exe" on Windows, and usually "the distro package is not installed"
//! on Linux. One row, three causes, and the suffix says which one to read
//! without having to ask the reporter what they run.
//!
//! Nothing here formats a code the user cannot look up: every code that can
//! be produced is in [`CODES`], and a test in this crate fails if
//! `docs/ERROR-CODES.md` has drifted from it.

use std::sync::Mutex;
use std::sync::OnceLock;

/// Which part of the program failed. The area is the first thing a
/// maintainer wants, so it is in the code itself rather than in the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    /// Encoding: the ffmpeg binary, its arguments, its encoders.
    Ffmpeg,
    /// The GPU: adapter, device, surface, shaders, memory.
    Gpu,
    /// Writing the output: disks, permissions, paths, file names.
    Output,
    /// Reading a replay, a map, a skin archive or the game's assets.
    MapReplay,
    /// Talking to the Rhythia API to fetch a map.
    Network,
    /// The app itself: settings, windows, state.
    App,
}

impl Area {
    pub const fn tag(self) -> &'static str {
        match self {
            Area::Ffmpeg => "FFM",
            Area::Gpu => "GPU",
            Area::Output => "OUT",
            Area::MapReplay => "MAP",
            Area::Network => "NET",
            Area::App => "APP",
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Area::Ffmpeg => "Encoding (ffmpeg)",
            Area::Gpu => "Graphics",
            Area::Output => "Writing the file",
            Area::MapReplay => "Replays and maps",
            Area::Network => "Downloads",
            Area::App => "Application",
        }
    }

    /// The code used when nothing in the area matched. Every area has one, so
    /// an unrecognised failure still leaves something to search the file for.
    const fn fallback(self) -> &'static str {
        match self {
            Area::Ffmpeg => "RH-FFM-100",
            Area::Gpu => "RH-GPU-200",
            Area::Output => "RH-OUT-300",
            Area::MapReplay => "RH-MAP-400",
            Area::Network => "RH-NET-500",
            Area::App => "RH-APP-600",
        }
    }
}

/// The operating systems the codes distinguish between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plat {
    Windows,
    Linux,
    MacOs,
    /// Same cause everywhere.
    Any,
}

impl Plat {
    pub const fn suffix(self) -> &'static str {
        match self {
            Plat::Windows => "W",
            Plat::Linux => "L",
            Plat::MacOs => "M",
            Plat::Any => "*",
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Plat::Windows => "Windows",
            Plat::Linux => "Linux",
            Plat::MacOs => "macOS",
            Plat::Any => "all systems",
        }
    }
}

/// The system this build runs on.
pub const fn current_plat() -> Plat {
    if cfg!(windows) {
        Plat::Windows
    } else if cfg!(target_os = "macos") {
        Plat::MacOs
    } else {
        Plat::Linux
    }
}

/// One row of the table: a failure, how to recognise it, what to tell the
/// user, and what it means on each system.
pub struct Entry {
    /// Without the platform suffix, which is added when a code is produced.
    pub id: &'static str,
    pub area: Area,
    /// What happened, in the user's words. Shown next to the code.
    pub title: &'static str,
    /// What the user can do next. Empty when there is nothing useful to say.
    pub fix: &'static str,
    /// Lowercase fragments of the raw error text that identify this failure.
    /// Empty means "fallback for the area", which is never matched against.
    pub patterns: &'static [&'static str],
    /// The maintainer-side cause, per system. This is the reason the codes
    /// carry a platform suffix at all.
    pub causes: &'static [(Plat, &'static str)],
}

impl Entry {
    /// The code as printed, for the system this build runs on.
    pub fn code(&self) -> String {
        format!("{}-{}", self.id, current_plat().suffix())
    }

    /// The cause for a given system, falling back to the shared one.
    pub fn cause_for(&self, plat: Plat) -> Option<&'static str> {
        self.causes
            .iter()
            .find(|(p, _)| *p == plat)
            .or_else(|| self.causes.iter().find(|(p, _)| *p == Plat::Any))
            .map(|(_, c)| *c)
    }
}

/// Every code rhythr can produce. Order matters inside an area: the first
/// entry whose pattern matches wins, so specific failures must come before
/// broad ones.
pub static CODES: &[Entry] = &[
    // ------------------------------------------------------- ffmpeg (FFM)
    Entry {
        id: "RH-FFM-101",
        area: Area::Ffmpeg,
        title: "ffmpeg could not be started",
        fix: "Point rhythr at an ffmpeg binary in Settings, or install one.",
        patterns: &[
            "could not start ffmpeg",
            "ffmpeg not found",
            "ffmpeg does not run",
        ],
        causes: &[
            (
                Plat::Windows,
                "No ffmpeg.exe on PATH and none next to rhythr.exe. The installer does not \
                 ship one, so this is the expected first-run failure on a bare Windows box.",
            ),
            (
                Plat::Linux,
                "The distro package is missing (the AppImage carries its own ffmpeg, the deb \
                 and rpm depend on the system one). Check that the binary in the setting is \
                 executable, and that a Flatpak or snap sandbox is not hiding it.",
            ),
            (
                Plat::MacOs,
                "Not installed (brew install ffmpeg), or Gatekeeper quarantined a hand-placed \
                 binary.",
            ),
        ],
    },
    Entry {
        id: "RH-FFM-102",
        area: Area::Ffmpeg,
        title: "the chosen encoder does not exist in this ffmpeg",
        fix: "Pick a different encoder in Settings; x264 works on every build.",
        patterns: &[
            "unknown encoder",
            "encoder not found",
            "could not find encoder",
            "unrecognized option",
        ],
        causes: &[
            (
                Plat::Any,
                "A build without the encoder compiled in. Distro builds routinely drop nvenc \
                 and qsv; the probe in video.rs is meant to catch this before a render, so a \
                 code here also means the probe and the render disagreed.",
            ),
            (
                Plat::Linux,
                "Also seen when ffmpeg is a Flatpak or snap wrapper whose feature set differs \
                 from the one the probe measured.",
            ),
        ],
    },
    Entry {
        id: "RH-FFM-103",
        area: Area::Ffmpeg,
        title: "the hardware encoder refused the job",
        fix: "Switch the encoder to x264, or update the graphics driver.",
        patterns: &[
            "cannot load nvcuda",
            "no capable devices found",
            "openencodesessionex failed",
            "no device available",
            "device creation failed",
            "failed to initialise vaapi",
            "no va display",
            "amf failed",
        ],
        causes: &[
            (
                Plat::Windows,
                "Driver too old for the ffmpeg build, or every NVENC session is taken (consumer \
                 drivers cap concurrent sessions, and a running stream or recording holds one).",
            ),
            (
                Plat::Linux,
                "VA-API needs /dev/dri access: a user outside the video and render groups gets \
                 exactly this. NVENC additionally needs the proprietary driver, not nouveau.",
            ),
            (
                Plat::MacOs,
                "NVENC, QSV and AMF do not exist here; only VideoToolbox and x264 do.",
            ),
        ],
    },
    Entry {
        id: "RH-FFM-104",
        area: Area::Ffmpeg,
        title: "ffmpeg stopped in the middle of the render",
        fix: "Try again with x264; if it repeats, save diagnostics and report it.",
        patterns: &[
            "writing frame",
            "broken pipe",
            "ffmpeg exited",
            "exited with status",
        ],
        causes: &[
            (
                Plat::Any,
                "ffmpeg died while frames were still coming. Its own last words are appended to \
                 the message and say more than the errno: out of memory, an unsupported pixel \
                 format, or a full disk are the usual three.",
            ),
            (
                Plat::Linux,
                "A kill by signal 9 with no stderr is the OOM killer, which at 4K is a real \
                 possibility on 8 GB machines.",
            ),
        ],
    },
    Entry {
        id: "RH-FFM-105",
        area: Area::Ffmpeg,
        title: "the frame socket never came up",
        fix: "Turn the socket transport off in Settings (the pipe always works).",
        patterns: &[
            "never connected to the frame socket",
            "could not arm the frame socket",
            "could not settle the frame socket",
            "frame socket failed",
        ],
        causes: &[
            (
                Plat::Windows,
                "A firewall or endpoint-protection product blocking a loopback listener. It is \
                 loopback only, but that is not a distinction every product makes.",
            ),
            (
                Plat::Linux,
                "Rare. A sandbox without loopback networking (Flatpak with no network \
                 permission) is the case that has been seen.",
            ),
            (
                Plat::MacOs,
                "The local network prompt was denied for rhythr.",
            ),
        ],
    },
    Entry {
        id: "RH-FFM-100",
        area: Area::Ffmpeg,
        title: "the encoder failed",
        fix: "Save diagnostics and report it; the ffmpeg output is in the file.",
        patterns: &[],
        causes: &[(
            Plat::Any,
            "Nothing in the FFM list matched. The raw ffmpeg text in the diagnostics file is \
             the whole diagnosis; if it turns out to be a repeating case, give it its own code.",
        )],
    },
    // ---------------------------------------------------------- GPU (GPU)
    Entry {
        id: "RH-GPU-201",
        area: Area::Gpu,
        title: "no usable graphics adapter",
        fix: "Update the graphics driver, or start rhythr with WGPU_BACKEND=gl.",
        patterns: &[
            "no compatible gpu adapter",
            "no usable gpu",
            "no graphics adapter",
            "no usable gpu adapter",
        ],
        causes: &[
            (
                Plat::Windows,
                "No DX12 and no Vulkan: a very old GPU, a machine running on the Microsoft \
                 Basic Display Adapter, or an RDP session with no GPU passed through.",
            ),
            (
                Plat::Linux,
                "The usual one. No Vulkan ICD installed (mesa-vulkan-drivers or the vendor \
                 driver), a headless session with no render node, or a user without access to \
                 /dev/dri (groups video and render).",
            ),
            (
                Plat::MacOs,
                "Effectively impossible on supported hardware; every Metal-capable Mac is fine.",
            ),
        ],
    },
    Entry {
        id: "RH-GPU-202",
        area: Area::Gpu,
        title: "the GPU was found but would not open a device",
        fix: "Update the graphics driver, or start rhythr with WGPU_BACKEND=gl.",
        patterns: &[
            "gpu device request failed",
            "would not open a device",
            "requestdevice",
        ],
        causes: &[
            (
                Plat::Any,
                "An adapter answered and then failed to hand out a device: usually a driver \
                 that is out of date or has been left wedged by an earlier crash.",
            ),
            (
                Plat::Linux,
                "Also a mismatch between a freshly updated kernel module and a still-loaded \
                 old one, which a reboot fixes and nothing else does.",
            ),
        ],
    },
    Entry {
        id: "RH-GPU-203",
        area: Area::Gpu,
        title: "the GPU ran out of memory",
        fix: "Render at a smaller resolution, or close other GPU-heavy programs.",
        patterns: &[
            "out of memory",
            "outofmemory",
            "device_removed",
            "allocation failed",
        ],
        causes: &[
            (
                Plat::Any,
                "4K with three readback slots is roughly 100 MB of VRAM on top of the scene. On \
                 a card that is already carrying a game or a browser with hardware acceleration, \
                 that is the frame that does not fit.",
            ),
            (
                Plat::Windows,
                "A DXGI_ERROR_DEVICE_REMOVED here is usually TDR: the driver reset because a \
                 single submission took longer than two seconds.",
            ),
        ],
    },
    Entry {
        id: "RH-GPU-200",
        area: Area::Gpu,
        title: "the graphics layer failed",
        fix: "Save diagnostics and report it; it names the GPU and the driver.",
        patterns: &[],
        causes: &[(
            Plat::Any,
            "Nothing in the GPU list matched. The gpu section of the diagnostics file has the \
             adapter, the backend and the driver version, which is normally enough.",
        )],
    },
    // ------------------------------------------------------- output (OUT)
    //
    // RH-OUT-303 comes first on purpose: a failed rename reports the same
    // "access is denied" as any other refused write, and the difference
    // matters here more than anywhere else, because in that one case the
    // render did finish and the file is sitting there under its part name.
    Entry {
        id: "RH-OUT-303",
        area: Area::Output,
        title: "the finished render could not be moved into place",
        fix: "The video is safe under its .rhythr-part name; close whatever has the target open.",
        patterns: &[
            "could not be moved into place",
            "being used by another process",
            "sharing violation",
        ],
        causes: &[
            (
                Plat::Windows,
                "The destination is open in a media player or being scanned by antivirus. \
                 Windows refuses the rename; the encode itself was fine.",
            ),
            (
                Plat::Any,
                "The part file and the destination are on different filesystems, or the \
                 destination folder disappeared during the render.",
            ),
        ],
    },
    Entry {
        id: "RH-OUT-301",
        area: Area::Output,
        title: "not allowed to write there",
        fix: "Pick a different output folder in Settings.",
        patterns: &[
            "permission denied",
            "access is denied",
            "os error 13",
            "os error 5",
        ],
        causes: &[
            (
                Plat::Windows,
                "Writing into Program Files, a drive root, or a OneDrive folder that is \
                 currently locked by sync.",
            ),
            (
                Plat::Linux,
                "A folder owned by root, or a read-only mount. Inside a Flatpak sandbox, any \
                 path outside the granted portals looks exactly like this.",
            ),
            (
                Plat::MacOs,
                "The app has not been granted access to Desktop, Documents or Downloads yet, \
                 which macOS gates behind a prompt that can be dismissed.",
            ),
        ],
    },
    Entry {
        id: "RH-OUT-302",
        area: Area::Output,
        title: "the disk is full",
        fix: "Free some space or choose another drive, then render again.",
        patterns: &[
            "no space left",
            "not enough space",
            "os error 28",
            "os error 112",
        ],
        causes: &[(
            Plat::Any,
            "A 4K render at high quality is easily several GB, and the part file lives beside \
             the final one, so the peak is one file, not two.",
        )],
    },
    Entry {
        id: "RH-OUT-304",
        area: Area::Output,
        title: "the output path is not usable",
        fix: "Choose the output folder again and keep the file name simple.",
        patterns: &[
            "no output folder set",
            "invalid argument",
            "filename, directory name, or volume label",
            "os error 123",
            "file name too long",
            "os error 36",
        ],
        causes: &[
            (
                Plat::Windows,
                "A character Windows forbids in a name (: * ? \" < > |), a reserved name like \
                 CON, or a path over 260 characters on a system without long paths enabled. \
                 Map titles supply all three regularly.",
            ),
            (
                Plat::Linux,
                "A name over 255 bytes, which a long song title in a non-Latin script reaches \
                 sooner than it looks.",
            ),
            (
                Plat::MacOs,
                "A colon in the name: HFS and APFS show it as a slash in Finder and reject it \
                 in the API.",
            ),
        ],
    },
    Entry {
        id: "RH-OUT-300",
        area: Area::Output,
        title: "the file could not be written",
        fix: "Try a different output folder, then save diagnostics if it repeats.",
        patterns: &[],
        causes: &[(
            Plat::Any,
            "Nothing in the OUT list matched. The output folder is listed (redacted) in the \
             diagnostics file.",
        )],
    },
    // ------------------------------------------- replays and maps (MAP)
    Entry {
        id: "RH-MAP-401",
        area: Area::MapReplay,
        title: "the replay file is damaged or truncated",
        fix: "Re-export the replay from the game and load it again.",
        patterns: &[
            "unexpected end of data",
            "malformed varint",
            "is not valid utf-8",
            "is not a valid length",
        ],
        causes: &[(
            Plat::Any,
            "The parser ran off the end of the file. A copy that was interrupted, a file pulled \
             out of a partially synced folder, or a replay the game itself never finished writing.",
        )],
    },
    Entry {
        id: "RH-MAP-402",
        area: Area::MapReplay,
        title: "the map file could not be read",
        fix: "Load the map again, or let rhythr download it from rhythia.com.",
        patterns: &[
            "sspm:",
            ".rhm archive",
            "map json:",
            "unsupported map file extension",
            "-byte limit",
        ],
        causes: &[(
            Plat::Any,
            "A map format rhythr does not know, a damaged archive, or an .sspm from a newer \
             version of the format than this build handles.",
        )],
    },
    Entry {
        id: "RH-MAP-403",
        area: Area::MapReplay,
        title: "the loaded map is not the one the replay was played on",
        fix: "Let rhythr download the right map, or load the exact file you played.",
        patterns: &["may not match", "does not match the replay", "wrong map"],
        causes: &[(
            Plat::Any,
            "Hash or map id disagree, or the hit pattern does not line up. This is a warning, \
             not a failure: the render still runs, and it is the reason a run can look \
             'manipulated' when the only mistake was the chart.",
        )],
    },
    Entry {
        id: "RH-MAP-404",
        area: Area::MapReplay,
        title: "the game files could not be read",
        fix: "Point rhythr at the game folder again in Settings.",
        patterns: &["game assets", "could not find the game", ".pck"],
        causes: &[
            (
                Plat::Windows,
                "The game was moved or reinstalled and the stored path is stale.",
            ),
            (
                Plat::Linux,
                "A Steam or Proton install whose real path sits under a compatdata folder that \
                 changes; also a Flatpak game whose files are outside rhythr's sandbox.",
            ),
            (
                Plat::MacOs,
                "The .app bundle was moved to the Bin, or the files are inside a bundle rhythr \
                 has not been granted access to.",
            ),
        ],
    },
    Entry {
        id: "RH-MAP-400",
        area: Area::MapReplay,
        title: "the replay or map could not be loaded",
        fix: "Save diagnostics and report it with the file if you can share it.",
        patterns: &[],
        causes: &[(
            Plat::Any,
            "Nothing in the MAP list matched. The loaded section of the diagnostics file names \
             the file (without the folder) and what was parsed out of it.",
        )],
    },
    // ------------------------------------------------------ network (NET)
    Entry {
        id: "RH-NET-501",
        area: Area::Network,
        title: "rhythia.com could not be reached",
        fix: "Check the internet connection, then press Download again.",
        patterns: &[
            "dns failed",
            "dns error",
            "failed to lookup",
            "no route to host",
            "connection refused",
            "network is unreachable",
            "os error 11001",
        ],
        causes: &[
            (
                Plat::Any,
                "No connection, or DNS is not answering. rhythr only ever contacts rhythia.com, \
                 so a working browser and a failing download usually means a proxy.",
            ),
            (
                Plat::Windows,
                "A corporate proxy or a VPN that captures DNS. ureq does not read the system \
                 proxy settings, so a machine that only reaches the internet through one fails \
                 here while every browser on it works.",
            ),
            (
                Plat::Linux,
                "A sandbox without network access (Flatpak), or a resolver that only exists \
                 inside the host namespace.",
            ),
        ],
    },
    Entry {
        id: "RH-NET-502",
        area: Area::Network,
        title: "rhythia.com is rate-limiting us",
        fix: "Wait a moment and press Download again.",
        patterns: &["rate-limiting", "429", "too many requests"],
        causes: &[(
            Plat::Any,
            "HTTP 429. rhythr makes one request per uncached map, so this is the server's own \
             limit being hit, not a loop on our side. If a code shows a high count, look for a \
             retry that is not backing off.",
        )],
    },
    Entry {
        id: "RH-NET-503",
        area: Area::Network,
        title: "rhythia.com is having trouble",
        fix: "Try again later, or load the map file by hand.",
        patterns: &["is unavailable right now", "status code 5", "server error"],
        causes: &[(
            Plat::Any,
            "An HTTP 5xx from the API. Nothing on the user's side to fix, and nothing on ours \
             either.",
        )],
    },
    Entry {
        id: "RH-NET-504",
        area: Area::Network,
        title: "the download timed out",
        fix: "Try again; if it keeps timing out, download the map in a browser.",
        patterns: &["timed out", "timeout"],
        causes: &[(
            Plat::Any,
            "10 s to connect, 60 s overall. A large map on a slow line can genuinely exceed \
             the second one.",
        )],
    },
    Entry {
        id: "RH-NET-505",
        area: Area::Network,
        title: "the secure connection could not be established",
        fix: "Check the system clock and any antivirus that inspects HTTPS.",
        patterns: &["certificate", "tls", "handshake", "invalid peer"],
        causes: &[
            (
                Plat::Windows,
                "Antivirus doing HTTPS inspection with its own root, which rhythr's bundled \
                 root store does not contain. A wrong system clock produces the same message.",
            ),
            (
                Plat::Linux,
                "A machine with no CA bundle at all, which happens in minimal containers.",
            ),
            (
                Plat::MacOs,
                "Usually the clock, occasionally a corporate MDM root.",
            ),
        ],
    },
    Entry {
        id: "RH-NET-506",
        area: Area::Network,
        title: "the map on the server is not the one the replay used",
        fix: "The map was re-uploaded; load the file you played if you still have it.",
        patterns: &["no beatmapfile", "does not parse", "hash mismatch"],
        causes: &[(
            Plat::Any,
            "The API answered but with something unusable: a map with no file, or a file that \
             does not parse. A re-upload under the same id is the common case, which is what \
             fetchedFor in the cache metadata exists to remember.",
        )],
    },
    Entry {
        id: "RH-NET-500",
        area: Area::Network,
        title: "the map could not be downloaded",
        fix: "Try again, or load the map file by hand.",
        patterns: &[],
        causes: &[(
            Plat::Any,
            "Nothing in the NET list matched. The raw message carries the ureq text, which \
             names the URL and the stage.",
        )],
    },
    // ----------------------------------------------------- the app (APP)
    Entry {
        id: "RH-APP-601",
        area: Area::App,
        title: "nothing is loaded yet",
        fix: "Load a replay first; the map follows from it.",
        patterns: &[
            "no replay loaded",
            "no map loaded",
            "replay has no online map id",
        ],
        causes: &[(
            Plat::Any,
            "A command that needs a replay ran without one. Reachable by keyboard shortcuts and \
             by the Analyze window before anything is loaded.",
        )],
    },
    Entry {
        id: "RH-APP-602",
        area: Area::App,
        title: "a render is already running",
        fix: "Wait for it to finish, or cancel it first.",
        patterns: &["rendering in progress", "already rendering"],
        causes: &[(
            Plat::Any,
            "Deliberate: benchmarks, preview renders and exports all refuse while a render \
             holds the GPU, because sharing it makes both slower and the timings meaningless.",
        )],
    },
    Entry {
        id: "RH-APP-603",
        area: Area::App,
        title: "the renderer crashed",
        fix: "Save diagnostics and report it; this one is always a bug.",
        patterns: &["renderer crashed", "panicked"],
        causes: &[(
            Plat::Any,
            "A panic on the render thread, caught so the app survives. The diagnostics file has \
             the GPU, the settings and the last render, which is what makes it reproducible.",
        )],
    },
    Entry {
        id: "RH-APP-604",
        area: Area::App,
        title: "the settings file could not be read or written",
        fix: "Settings will fall back to defaults; check the config folder's permissions.",
        patterns: &["could not save settings", "settings file", "config dir"],
        causes: &[
            (
                Plat::Windows,
                "%APPDATA%\\rhythr is on a roaming profile that is not available, or is being \
                 held by another copy of rhythr.",
            ),
            (
                Plat::Linux,
                "$XDG_CONFIG_HOME points somewhere unwritable, or $HOME is not set at all \
                 (which happens when rhythr is launched from a service unit).",
            ),
            (
                Plat::MacOs,
                "~/Library/Application Support is not writable, which sandboxing or a migrated \
                 user account can cause.",
            ),
        ],
    },
    Entry {
        id: "RH-APP-600",
        area: Area::App,
        title: "something went wrong",
        fix: "Save diagnostics and report it.",
        patterns: &[],
        causes: &[(
            Plat::Any,
            "The catch-all. Nothing anywhere in the table matched, so the message itself is the \
             only lead. A code that shows up often here is a missing row.",
        )],
    },
];

/// Looks a code up by its id, with or without a platform suffix.
pub fn find(code: &str) -> Option<&'static Entry> {
    let base = code
        .strip_suffix("-W")
        .or_else(|| code.strip_suffix("-L"))
        .or_else(|| code.strip_suffix("-M"))
        .unwrap_or(code);
    CODES.iter().find(|e| e.id.eq_ignore_ascii_case(base))
}

/// Classifies a raw error message inside a known area.
pub fn classify_in(area: Area, message: &str) -> &'static Entry {
    let low = message.to_ascii_lowercase();
    CODES
        .iter()
        .filter(|e| e.area == area && !e.patterns.is_empty())
        .find(|e| e.patterns.iter().any(|p| low.contains(p)))
        .unwrap_or_else(|| fallback(area))
}

/// Classifies a raw error message with no idea where it came from, which is
/// the situation at the boundary where any command's error reaches the UI.
pub fn classify(message: &str) -> &'static Entry {
    let low = message.to_ascii_lowercase();
    CODES
        .iter()
        .filter(|e| !e.patterns.is_empty())
        .find(|e| e.patterns.iter().any(|p| low.contains(p)))
        .unwrap_or_else(|| fallback(Area::App))
}

fn fallback(area: Area) -> &'static Entry {
    CODES
        .iter()
        .find(|e| e.id == area.fallback())
        .expect("every area needs a fallback entry")
}

/// Classifies, records, and returns the message with its code appended.
///
/// This is what the app calls: a coded message is the only kind the user
/// should ever see, because an uncoded one cannot be looked up.
pub fn stamp(message: &str) -> String {
    stamp_in_area(None, message)
}

/// Same, for a caller that knows which area it is in. Preferred where the
/// context is known, since a bare "permission denied" is otherwise ambiguous.
pub fn stamp_in(area: Area, message: &str) -> String {
    stamp_in_area(Some(area), message)
}

/// Outcomes that travel as errors because that is how they unwind, but that
/// nobody would report: the user asked for them. A code on "render cancelled"
/// would be noise in the message and, worse, noise in the diagnostics list
/// that is meant to be read top to bottom.
fn is_not_a_failure(message: &str) -> bool {
    let low = message.to_ascii_lowercase();
    low.contains("cancelled") || low.contains("canceled")
}

fn stamp_in_area(area: Option<Area>, message: &str) -> String {
    if is_not_a_failure(message) {
        return message.to_string();
    }
    let entry = match area {
        Some(a) => classify_in(a, message),
        None => classify(message),
    };
    let code = entry.code();
    record_code(&code, message);
    // Already stamped (an inner layer got there first): do not stack codes.
    if message.contains("[RH-") {
        return message.to_string();
    }
    format!("{message} [{code}]")
}

/// One recorded failure, for the diagnostics report.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub code: String,
    /// The message as it was, minus the code, and trimmed to
    /// [`MESSAGE_MAX`]. Kept whole rather than shortened to its first line,
    /// because ffmpeg puts its reason at the END and that reason is the
    /// single most useful line in the file. May contain a path, so whoever
    /// prints this is responsible for redacting it.
    pub message: String,
    /// How often this code has come up since the app started.
    pub count: u32,
}

const RECENT_MAX: usize = 24;
/// Long enough for an ffmpeg failure with its stderr tail, short enough that
/// a render looping on an error cannot grow the report without bound.
const MESSAGE_MAX: usize = 600;

fn recent_store() -> &'static Mutex<Vec<Recorded>> {
    static RECENT: OnceLock<Mutex<Vec<Recorded>>> = OnceLock::new();
    RECENT.get_or_init(|| Mutex::new(Vec::new()))
}

fn record_code(code: &str, message: &str) {
    let mut text = message.trim().to_string();
    if text.len() > MESSAGE_MAX {
        // On a char boundary: a message can carry a map title in any script.
        let mut cut = MESSAGE_MAX;
        while cut > 0 && !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str(" [...]");
    }
    let Ok(mut list) = recent_store().lock() else {
        return;
    };
    if let Some(hit) = list.iter_mut().find(|r| r.code == code) {
        hit.count += 1;
        hit.message = text;
        return;
    }
    if list.len() == RECENT_MAX {
        list.remove(0);
    }
    list.push(Recorded {
        code: code.to_string(),
        message: text,
        count: 1,
    });
}

/// Everything that has gone wrong since the app started, oldest first.
pub fn recent() -> Vec<Recorded> {
    recent_store().lock().map(|l| l.clone()).unwrap_or_default()
}

/// The maintainer-side lookup table as markdown, so `docs/ERROR-CODES.md` is
/// generated from the same list the app classifies with and cannot drift.
pub fn markdown() -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    s.push_str(
        "# rhythr error codes\n\n\
         Generated from `crates/errcode/src/lib.rs`. Do not edit by hand: run\n\
         `RHYTHR_UPDATE_DOCS=1 cargo test -p rhythia-errcode` after changing the table.\n\n\
         A code looks like `RH-FFM-101-L`. The last letter is the system it came from\n\
         (`W`indows, `L`inux, `M`acOS), because the same failure usually has a different\n\
         cause on each. Codes are stable: a row is never renumbered or reused, and a\n\
         failure that stops existing keeps its row marked as retired.\n\n\
         Users find these codes in the app next to the error, and at the top of the file\n\
         written by Settings > Diagnostics.\n",
    );
    for area in [
        Area::Ffmpeg,
        Area::Gpu,
        Area::Output,
        Area::MapReplay,
        Area::Network,
        Area::App,
    ] {
        let _ = write!(s, "\n## {} ({})\n", area.title(), area.tag());
        for e in CODES.iter().filter(|e| e.area == area) {
            let _ = write!(s, "\n### {} {}\n\n", e.id, e.title);
            if !e.fix.is_empty() {
                let _ = write!(s, "Shown to the user: {}\n\n", e.fix);
            }
            for (plat, cause) in e.causes {
                let _ = write!(s, "- **{}**: {}\n", plat.name(), cause);
            }
            if !e.patterns.is_empty() {
                let _ = write!(
                    s,
                    "- Recognised by: {}\n",
                    e.patterns
                        .iter()
                        .map(|p| format!("`{p}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn docs_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/ERROR-CODES.md")
    }

    /// The table in the app and the file a maintainer reads must be the same
    /// list. Set RHYTHR_UPDATE_DOCS=1 to regenerate the file.
    #[test]
    fn docs_match_the_table() {
        let want = markdown();
        let path = docs_path();
        if std::env::var("RHYTHR_UPDATE_DOCS").is_ok() {
            std::fs::write(&path, &want).expect("write docs");
            return;
        }
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(
            have, want,
            "docs/ERROR-CODES.md is out of date, run RHYTHR_UPDATE_DOCS=1 cargo test -p rhythia-errcode"
        );
    }

    /// A code the app can produce but nobody can look up is worse than none.
    #[test]
    fn every_area_has_a_fallback() {
        for area in [
            Area::Ffmpeg,
            Area::Gpu,
            Area::Output,
            Area::MapReplay,
            Area::Network,
            Area::App,
        ] {
            let f = fallback(area);
            assert_eq!(f.area, area);
            assert!(f.patterns.is_empty(), "a fallback must not be matchable");
        }
    }

    #[test]
    fn ids_are_unique_and_well_formed() {
        let mut seen = std::collections::BTreeSet::new();
        for e in CODES {
            assert!(seen.insert(e.id), "duplicate id {}", e.id);
            let parts: Vec<_> = e.id.split('-').collect();
            assert_eq!(parts.len(), 3, "malformed id {}", e.id);
            assert_eq!(parts[0], "RH");
            assert_eq!(parts[1], e.area.tag(), "id area disagrees with the entry");
            assert!(parts[2].len() == 3 && parts[2].chars().all(|c| c.is_ascii_digit()));
        }
    }

    /// The messages below are copied from the code that produces them. A
    /// classifier tested against invented strings tests nothing: it is the
    /// real wording that has to keep matching, and it is the real wording
    /// that changes when someone rewords an error.
    #[test]
    fn real_messages_land_on_the_right_code() {
        let cases: &[(&str, &str)] = &[
            (
                "video export failed: could not start ffmpeg (ffmpeg): No such file or \
                 directory (os error 2)",
                "RH-FFM-101",
            ),
            (
                "video export failed: writing frame 812 failed: Broken pipe (os error 32) \
                 (ffmpeg exited with status 1)",
                "RH-FFM-104",
            ),
            (
                "video export failed: ffmpeg never connected to the frame socket",
                "RH-FFM-105",
            ),
            (
                "no usable GPU: no graphics adapter accepted the renderer.",
                "RH-GPU-201",
            ),
            (
                "the GPU was found but would not open a device (device lost).",
                "RH-GPU-202",
            ),
            (
                "the render finished but could not be moved into place (Access is denied. \
                 (os error 5))",
                "RH-OUT-303",
            ),
            (
                "failed to write output: No space left on device (os error 28)",
                "RH-OUT-302",
            ),
            (
                "unexpected end of data at byte 4096 (needed 12 more)",
                "RH-MAP-401",
            ),
            (".sspm map: sspm: bad signature", "RH-MAP-402"),
            (
                "rhythia.com is rate-limiting requests, please wait a moment and press \
                 Download again",
                "RH-NET-502",
            ),
            (
                "map lookup failed: https://rhythia.com/api/...: Dns Failed: resolve dns name",
                "RH-NET-501",
            ),
            ("renderer crashed: index out of bounds", "RH-APP-603"),
        ];
        for (message, want) in cases {
            let got = classify(message).id;
            assert_eq!(got, *want, "{message:?} classified as {got}");
        }
    }

    /// A "permission denied" from a download and from a render are different
    /// problems with the same words, which is what the area hint is for.
    #[test]
    fn the_area_hint_beats_the_words() {
        assert_eq!(
            classify_in(Area::Network, "connection refused").id,
            "RH-NET-501"
        );
        assert_eq!(
            classify_in(Area::Output, "connection refused").id,
            "RH-OUT-300",
            "an unmatched message inside an area falls back inside that area"
        );
    }

    #[test]
    fn a_code_is_never_stacked_twice() {
        let once = stamp("something went wrong");
        let twice = stamp(&once);
        assert_eq!(once, twice);
    }

    /// Cancelling is not a failure, and a session full of "RH-APP-600" from
    /// people pressing Cancel would bury the one line that matters.
    #[test]
    fn cancelling_is_not_recorded_as_a_failure() {
        assert_eq!(stamp("render cancelled"), "render cancelled");
        // Not is_empty(): the record is process-wide and the other tests in
        // this binary run beside this one, so the claim has to be about this
        // message rather than about the whole list.
        assert!(recent().iter().all(|r| !r.message.contains("cancelled")));
    }
}

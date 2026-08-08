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
        fix: "Point rhythr at an ffmpeg binary under Advanced, or install one.",
        patterns: &[
            "could not start ffmpeg",
            "ffmpeg not found",
            "ffmpeg does not run",
            "ffmpeg could not be run",
        ],
        causes: &[
            (
                Plat::Windows,
                "The installer puts ffmpeg.exe next to the app and the PATH is not consulted, so \
                 this means that copy is gone: deleted, quarantined by antivirus (static GPL \
                 builds are a frequent false positive), blocked by SmartScreen or AppLocker (os \
                 error 5), or the app folder was copied without its resources.",
            ),
            (
                Plat::Linux,
                "The distro package is missing: the AppImage carries its own copy, but deb and \
                 rpm only depend on one, so a dpkg -i without dependencies or a Fedora install \
                 without RPM Fusion leaves nothing to run. A dangling symlink or a \
                 non-executable stub on PATH shadows the bundled copy and reads the same way.",
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
        fix: "Set the encoder to Auto, which only offers encoders that passed a real test.",
        patterns: &[
            "unknown encoder",
            "encoder not found",
            "could not find encoder",
            "unrecognized option",
        ],
        causes: &[
            (
                Plat::Any,
                "A build without that encoder compiled in. Auto probes with a real test encode \
                 and falls back, so a code here means the encoder was forced (the CLI does not \
                 probe a forced choice) or the ffmpeg path changed after the probe, which is \
                 cached per path for the life of the process.",
            ),
            (
                Plat::Linux,
                "Fedora and RHEL ship ffmpeg-free, and openSUSE a patent-free build, with no \
                 libx264 and no nvenc, and the rpm's dependency is satisfied by it. Enable RPM \
                 Fusion and install the full ffmpeg, or use the AppImage.",
            ),
            (
                Plat::Windows,
                "Only after the ffmpeg path was repointed at a minimal or LGPL build. The \
                 bundled one always carries libx264, nvenc, qsv and amf.",
            ),
        ],
    },
    Entry {
        id: "RH-FFM-103",
        area: Area::Ffmpeg,
        title: "the hardware encoder refused the job",
        fix: "Switch the encoder to x264, or update the graphics driver and restart rhythr.",
        patterns: &[
            "cannot load nvcuda",
            "cannot load libcuda",
            "no capable devices found",
            "openencodesessionex failed",
            "no device available",
            "device creation failed",
            "failed to initialise vaapi",
            "no va display",
            "no working vaapi",
            "amf failed",
        ],
        causes: &[
            (
                Plat::Windows,
                "The driver is older than the encoder API this ffmpeg was built against, the \
                 iGPU is disabled in the BIOS (QSV), AMF is missing because the driver came from \
                 Windows Update rather than Adrenalin, or every NVENC session is taken (consumer \
                 drivers cap them and a running stream or recording holds one). All of them also \
                 fail inside RDP.",
            ),
            (
                Plat::Linux,
                "VAAPI needs a readable /dev/dri render node, so a user outside the render and \
                 video groups gets exactly this; it also needs the libva driver \
                 (mesa-va-drivers, intel-media-va-driver). NVENC needs the proprietary driver \
                 plus libnvidia-encode, which nouveau does not have. AMD does not ship AMF on \
                 Linux at all, so an AMF line in the diagnostics is expected there.",
            ),
            (
                Plat::MacOs,
                "NVENC, QSV and AMF do not exist here. Only VideoToolbox and x264 do.",
            ),
        ],
    },
    Entry {
        id: "RH-FFM-104",
        area: Area::Ffmpeg,
        title: "the encoder cannot handle this frame size",
        fix: "Render at 4K or below, or choose the x264 software encoder.",
        patterns: &[
            "does not support encoding at size",
            "hardware does not support",
            "width not divisible",
            "not divisible by 2",
        ],
        causes: &[(
            Plat::Any,
            "Every consumer H.264 hardware encoder tops out at 4096x4096, on all three systems \
             and every vendor. The availability probe encodes 256x256, so it cannot see this \
             coming. 8K is a software-encoder job.",
        )],
    },
    Entry {
        id: "RH-FFM-105",
        area: Area::Ffmpeg,
        title: "ffmpeg stopped in the middle of the render",
        fix: "Check free space, then try again with x264; save diagnostics if it repeats.",
        patterns: &[
            "writing frame",
            "broken pipe",
            "ffmpeg exited",
            "exited with status",
            "stopped by signal",
        ],
        causes: &[
            (
                Plat::Any,
                "ffmpeg died while frames were still coming. Its own last words are appended to \
                 the message and are worth more than the errno in front of them. A status 1 \
                 within a second of starting is not a full disk: run the resolved ffmpeg with \
                 -version by hand.",
            ),
            (
                Plat::Linux,
                "Signal 9 with an empty stderr is the OOM killer, which 4K reaches on 8 GB \
                 machines. Signal 25 is a file size limit: a FAT32 or exFAT output disk stops at \
                 4 GB, and ulimit -f does the same.",
            ),
            (
                Plat::Windows,
                "Antivirus or Controlled folder access terminating ffmpeg.exe mid-encode, a \
                 driver reset taking the encoder session with it, the output drive \
                 disconnecting, or the machine sleeping. Signal names do not exist here, so the \
                 message ends with a bare status number.",
            ),
        ],
    },
    Entry {
        id: "RH-FFM-106",
        area: Area::Ffmpeg,
        title: "the frame socket never came up",
        fix: "Turn off \"send frames over a local connection\" under Advanced; the pipe always works.",
        patterns: &[
            "never connected to the frame socket",
            "could not arm the frame socket",
            "could not settle the frame socket",
            "frame socket failed",
        ],
        causes: &[
            (
                Plat::Windows,
                "A security suite intercepting loopback connections (Windows Firewall itself \
                 does not filter loopback, which is why the listener binds 127.0.0.1 \
                 explicitly). Some let the small probe through and block the real connection.",
            ),
            (
                Plat::Linux,
                "Rare: an ffmpeg built with --disable-network, a Flatpak or Snap ffmpeg without \
                 network permission, an AppArmor or SELinux policy, or a rule on lo.",
            ),
            (Plat::MacOs, "The local network prompt was denied for rhythr."),
        ],
    },
    Entry {
        id: "RH-FFM-107",
        area: Area::Ffmpeg,
        title: "every frame was accepted and no file came out",
        fix: "Free about one more video's worth of space and render again; a local disk beats a share.",
        patterns: &["nothing was written into output file", "error while filtering"],
        causes: &[(
            Plat::Any,
            "The encode reached the end and the muxer failed at the last step. The output is \
             written with +faststart, which rewrites the finished file once, so a disk with room \
             for exactly one copy runs out during the rewrite. A network share dropping, or an \
             x264 preset from a hand-edited settings file, do the same.",
        )],
    },
    Entry {
        id: "RH-FFM-100",
        area: Area::Ffmpeg,
        title: "the encoder failed",
        fix: "Save diagnostics and report it; ffmpeg's own output is in the file.",
        patterns: &[],
        causes: &[(
            Plat::Any,
            "Nothing in the FFM list matched. The message carries ffmpeg's first diagnostic line \
             and its last two, which is the whole diagnosis; if the same text turns up twice, \
             give it a row.",
        )],
    },
    // ---------------------------------------------------------- GPU (GPU)
    Entry {
        id: "RH-GPU-201",
        area: Area::Gpu,
        title: "no usable graphics adapter",
        fix: "Update the graphics driver. WGPU_BACKEND=gl forces the OpenGL backend.",
        patterns: &[
            "no compatible gpu adapter",
            "no usable gpu",
            "no graphics adapter",
            "no usable gpu adapter",
        ],
        causes: &[
            (
                Plat::Windows,
                "No DX12 and no Vulkan adapter was enumerated: only the Microsoft Basic Display \
                 Adapter is installed, the driver predates feature level 11_0, or the session has \
                 no GPU (RDP, a service context, a VM without paravirtualisation).",
            ),
            (
                Plat::Linux,
                "The common one, and usually not the user's fault: no Vulkan ICD is installed \
                 (mesa-vulkan-drivers, vulkan-radeon, vulkan-intel, nvidia-utils), and none of \
                 the packages depend on a driver ICD. Also /dev/dri unreadable without the \
                 render and video groups, a headless or SSH session, or a sandbox without the \
                 GPU socket.",
            ),
            (
                Plat::MacOs,
                "Effectively impossible on supported hardware: every Metal-capable Mac works.",
            ),
        ],
    },
    Entry {
        id: "RH-GPU-202",
        area: Area::Gpu,
        title: "the GPU was found but would not open a device",
        fix: "Reboot once, then update the graphics driver.",
        patterns: &["gpu device request failed", "would not open a device", "requestdevice"],
        causes: &[
            (
                Plat::Any,
                "An adapter answered and then failed to hand out a device at wgpu's default \
                 limits (8192 textures, 128 MiB storage bindings). A driver left wedged by an \
                 earlier GPU reset is the usual cause, and a reboot is the usual fix.",
            ),
            (
                Plat::Linux,
                "Often the OpenGL fallback: wgpu falls back to GL when no Vulkan ICD is present, \
                 and old Mesa GL reports limits below the defaults. Installing a real ICD is the \
                 fix, not a setting. Also a kernel and Mesa mismatch after a partial upgrade, \
                 which only a reboot clears.",
            ),
        ],
    },
    Entry {
        id: "RH-GPU-203",
        area: Area::Gpu,
        title: "the GPU ran out of memory",
        fix: "Drop one resolution step and close other GPU-heavy programs.",
        patterns: &["out of memory", "outofmemory", "not enough memory", "allocation failed"],
        causes: &[
            (
                Plat::Any,
                "A renderer at 8K wants roughly 0.8 to 1 GB before any skin or background: three \
                 readback buffers of about 127 MB each, three NV12 buffers, plus the targets. \
                 The preview renderer keeps its own device alive during an export and a live \
                 Analyze session is a third. RHYTHR_NO_GPU_NV12=1 frees part of it, slower.",
            ),
            (
                Plat::Windows,
                "Windows pages GPU memory to system RAM instead of failing, so a low-VRAM card \
                 more often crawls than crashes. When it does fail it says device removed.",
            ),
            (
                Plat::Linux,
                "Most drivers do not page out: the allocation fails outright, or the process is \
                 killed by the OOM killer when the driver backs the buffers with system RAM.",
            ),
        ],
    },
    Entry {
        id: "RH-GPU-204",
        area: Area::Gpu,
        title: "the GPU was lost part way through the render",
        fix: "Re-run at a lower resolution or frame rate, then update the graphics driver.",
        patterns: &[
            "device lost",
            "device_removed",
            "device removed",
            "wait timed out",
            "surface lost",
            "surface validation",
        ],
        causes: &[
            (
                Plat::Windows,
                "The watchdog resets the GPU when a submission does not return in about two \
                 seconds, which a 4K frame on a mid-range or laptop card can reach. Also a \
                 vendor driver updating itself during a render, or an overclock.",
            ),
            (
                Plat::Linux,
                "A kernel-level GPU hang or reset (amdgpu \"GPU reset begin\", i915 \"GPU HANG\", \
                 an nvidia Xid), a driver module reload, or eviction pressure from another GPU \
                 program. dmesg right after the failure names the real one.",
            ),
        ],
    },
    Entry {
        id: "RH-GPU-205",
        area: Area::Gpu,
        title: "the driver rejected one of rhythr's shaders",
        fix: "Update the driver. RHYTHR_NO_GPU_NV12=1 removes the compute shader meanwhile.",
        patterns: &["shader module", "shader validation", "spir-v", "dxil", "pipeline creation"],
        causes: &[
            (
                Plat::Windows,
                "A driver whose shader compiler rejects a pipeline (older Intel integrated \
                 drivers are the usual suspects), or a broken driver install.",
            ),
            (
                Plat::Linux,
                "A Mesa or RADV version with a SPIR-V regression, or the GL fallback backend, \
                 where the NV12 compute shader needs GLES 3.1-class compute that an older stack \
                 does not provide.",
            ),
        ],
    },
    Entry {
        id: "RH-GPU-200",
        area: Area::Gpu,
        title: "the graphics layer failed",
        fix: "Save diagnostics and report it; it names the GPU, the backend and the driver.",
        patterns: &[],
        causes: &[(
            Plat::Any,
            "Nothing in the GPU list matched. Read the gpu line of the diagnostics file first: a \
             device type of Cpu, or a name like llvmpipe, lavapipe or Microsoft Basic Render \
             Driver, means the render was on the processor and the complaint is really about \
             speed.",
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
        fix: "The .rhythr-part.mp4 file IS the video: close whatever holds the target open, or rename it.",
        patterns: &[
            "could not be moved into place",
            "being used by another process",
            "sharing violation",
        ],
        causes: &[
            (
                Plat::Windows,
                "The dominant case: the destination is open in a media player, Explorer's \
                 preview or thumbnail handler, a sync client or an antivirus scan, and Windows \
                 cannot replace an open file. A read-only attribute on the existing file does it \
                 too. The encode itself was fine.",
            ),
            (
                Plat::Any,
                "Elsewhere this is nearly unreachable, since rename replaces a file others have \
                 open. It needs the destination name to be a directory, an immutable file, a \
                 full or read-only filesystem, or a mount without atomic replace.",
            ),
        ],
    },
    Entry {
        id: "RH-OUT-301",
        area: Area::Output,
        title: "not allowed to write there",
        fix: "Try the home folder first: that says whether it is the folder or the app.",
        patterns: &["permission denied", "access is denied", "os error 13", "os error 5"],
        causes: &[
            (
                Plat::Windows,
                "Defender's Controlled folder access protects Videos, Documents and Desktop by \
                 default, and Videos is the default output folder, so a stock install can hit \
                 this with nothing done wrong. Allow-listing rhythr.exe alone does NOT help: \
                 ffmpeg is a separate process and needs its own entry. Also a read-only share, \
                 a sync folder over quota, or Program Files without elevation.",
            ),
            (
                Plat::Linux,
                "A folder owned by root because it was created once with sudo, a read-only mount \
                 (an NTFS or exFAT partition auto-mounted ro after an unclean Windows shutdown), \
                 a gvfs or MTP mount, or Flatpak confinement, where any path outside the granted \
                 portals looks exactly like this.",
            ),
            (
                Plat::MacOs,
                "Access to Desktop, Documents or Downloads has not been granted yet, which macOS \
                 gates behind a prompt that can be dismissed.",
            ),
        ],
    },
    Entry {
        id: "RH-OUT-302",
        area: Area::Output,
        title: "the disk is full",
        fix: "Free about twice the expected video size, or pick a folder on a bigger drive.",
        patterns: &["no space left", "not enough space", "os error 28", "os error 112", "quota"],
        causes: &[(
            Plat::Any,
            "A full-length 1080p60 render is several GB, and the file is rewritten once at the \
             end (+faststart), so the peak is about two copies. Look in the output folder for \
             abandoned .rhythr-part.mp4 files as well: each is a full-length partial encode, \
             nothing ever sweeps them, and a hard kill or a power cut leaves one behind.",
        )],
    },
    Entry {
        id: "RH-OUT-304",
        area: Area::Output,
        title: "the output path is not usable",
        fix: "Type a shorter file name, or render to a shallow folder.",
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
                "The 260-character path limit. A deep sync folder plus an auto-derived \
                 \"Player - Song (1.02-1.34).mp4\" can leave the final name fitting and the \
                 .rhythr-part sibling (12 characters longer) not, which is why the failure can \
                 look like a folder that plainly exists. ffmpeg is not long-path aware, so \
                 enabling long paths does not rescue it. Forbidden characters and reserved names \
                 like CON land here too.",
            ),
            (
                Plat::Linux,
                "The other half of the same problem: the name is capped at 150 CHARACTERS, and a \
                 Japanese, Korean or Cyrillic title is 2 to 3 bytes per character, so it can pass \
                 the cap and still exceed the 255-BYTE limit of ext4, xfs and btrfs.",
            ),
            (
                Plat::MacOs,
                "A colon in the name: Finder shows it as a slash and the API rejects it.",
            ),
        ],
    },
    Entry {
        id: "RH-OUT-300",
        area: Area::Output,
        title: "the file could not be written",
        fix: "Save into the home folder and try again.",
        patterns: &[],
        causes: &[(
            Plat::Any,
            "Nothing in the OUT list matched. The output folder is in the diagnostics file \
             (redacted). Note the Copy button next to Save diagnostics, which produces the same \
             report without touching the disk: useful when this is the failure being reported.",
        )],
    },
    // ------------------------------------------- replays and maps (MAP)
    Entry {
        id: "RH-MAP-401",
        area: Area::MapReplay,
        title: "the replay file is damaged, truncated, or not a replay",
        fix: "Re-export the replay from the game and copy the whole file.",
        patterns: &[
            "unexpected end of data",
            "malformed varint",
            "is not valid utf-8",
            "is not a valid length",
        ],
        causes: &[
            (
                Plat::Any,
                "There is no signature check on .rhr, so a file that is not a replay at all \
                 (a map, a skin export, the game's config.json, a renamed PNG) produces the same \
                 byte-level complaint as a truncated one. A healthy .rhr is a header plus a \
                 multiple of 17 bytes.",
            ),
            (
                Plat::Windows,
                "Explorer hides known extensions, so run.rhr.txt looks like run.rhr, and \
                 drag-and-drop accepts anything the Browse dialog would filter out. A copy \
                 interrupted mid-write, an unhydrated cloud placeholder, or the game still \
                 holding the file open all give a short file.",
            ),
            (
                Plat::Linux,
                "Usually a copy made out of the Proton prefix while the game was still writing \
                 it: there is no mandatory locking, so a mid-write read always succeeds \
                 partially and always lands here.",
            ),
        ],
    },
    Entry {
        id: "RH-MAP-402",
        area: Area::MapReplay,
        title: "the map file could not be read",
        fix: "Load the .sspm or .rhm itself, or let rhythr download the map.",
        patterns: &[
            "sspm:",
            ".rhm archive",
            "map json:",
            "unsupported map file extension",
            "-byte limit",
            "eocd",
        ],
        causes: &[(
            Plat::Any,
            "Only .sspm, .rhm and the game's cache .json are accepted, so a downloaded .zip \
             fails on its extension. \"Could not find EOCD\" means the archive is incomplete \
             (interrupted download); \"unsupported version 3\" means the map is newer than this \
             build; \"missing or invalid field Notes\" is what the game's own config.json \
             produces, because a dropped .json is tried as a map before it is tried as a skin.",
        )],
    },
    Entry {
        id: "RH-MAP-403",
        area: Area::MapReplay,
        title: "the loaded map is not the one the replay was played on",
        fix: "Let rhythr download the map by the replay's own id, or browse for the exact chart.",
        patterns: &["may not match", "does not match the replay", "wrong map"],
        causes: &[(
            Plat::Any,
            "Hash or map id disagree, or most recorded hits find no note. Rendering is not \
             blocked and the video carries no warning, so the render can silently show the wrong \
             notes. When neither heuristic fires, the same situation reads as \"possibly \
             manipulated\", which accuses the player for somebody else's map file. A locally \
             browsed map is not hash-checked at all.",
        )],
    },
    Entry {
        id: "RH-MAP-404",
        area: Area::MapReplay,
        title: "the game files could not be read",
        fix: "Use Locate and pick the real game binary (about 280 MB), not a shortcut.",
        patterns: &[
            "game assets",
            "could not find the game",
            ".pck",
            "no game resources",
            "not found in any steam library",
            "no usable skin assets",
        ],
        causes: &[
            (
                Plat::Windows,
                "Detect looks at the Steam registry key and the two Program Files defaults, so a \
                 non-Steam, itch or manual install is invisible. It picks the LARGEST matching \
                 binary, so a launcher stub or an old Sound Space Plus install can win. The \
                 extraction writes about 600 files and then swaps the folder, which fails while \
                 antivirus, the search indexer or a second rhythr holds one of them open.",
            ),
            (
                Plat::Linux,
                "Detect knows five home paths only (.local/share/Steam, .steam/steam, \
                 .steam/root, the Flatpak and Snap paths), so a distro-package Steam with data \
                 elsewhere, a system-wide install or a library on a root-only NTFS mount is \
                 invisible, and there is no registry to fall back on. A failed swap means the \
                 data directory is full, read-only, or root-owned from an earlier sudo run.",
            ),
            (
                Plat::MacOs,
                "The bundle was moved, or its files are somewhere rhythr has not been granted \
                 access to.",
            ),
        ],
    },
    Entry {
        id: "RH-MAP-405",
        area: Area::MapReplay,
        title: "the replay's timestamps are damaged",
        fix: "Re-export the replay from the game, or clip a shorter range to render it anyway.",
        patterns: &["which no run is", "claims to be"],
        causes: &[(
            Plat::Any,
            "A replay's length is the timestamp of its last frame, taken as read, and one \
             damaged stamp is enough to make that days. It passes every integrity check (the \
             header agrees with the frames), and the render it asks for is a progress bar that \
             never moves and a file that grows until the disk is full, which is why it is \
             refused before it starts.",
        )],
    },
    Entry {
        id: "RH-MAP-400",
        area: Area::MapReplay,
        title: "the replay or map could not be loaded",
        fix: "Save diagnostics and report it, with the file if you can share it.",
        patterns: &[],
        causes: &[(
            Plat::Any,
            "Nothing in the MAP list matched. A replay recorded by a NEWER game version parses \
             with today's field order and is then reported as inconsistent or broken, never as \
             \"rhythr is too old\", so check the version in the report before believing any \
             verdict.",
        )],
    },
    // ------------------------------------------------------ network (NET)
    Entry {
        id: "RH-NET-501",
        area: Area::Network,
        title: "rhythia.com could not be reached",
        fix: "Check the connection, then press Download again. A local map file always works.",
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
                "IMPORTANT: rhythr does not read the system proxy settings at all, so on a \
                 proxy-only school or company network every request fails here while the browser \
                 on the same machine works. There is no proxy setting in the app; the way \
                 through is to download the .sspm in a browser and load it with Browse.",
            ),
            (
                Plat::Windows,
                "Also a firewall or security suite blocking the freshly installed unsigned \
                 rhythr.exe outbound, or plain DNS failure.",
            ),
            (
                Plat::Linux,
                "Also systemd-resolved down, a broken /etc/resolv.conf, an egress firewall rule, \
                 or a sandbox without network.",
            ),
        ],
    },
    Entry {
        id: "RH-NET-502",
        area: Area::Network,
        title: "rhythia.com is rate-limiting us",
        fix: "Wait a minute and press Download once. Repeated clicks make it worse.",
        patterns: &["rate-limiting", "429", "too many requests"],
        causes: &[(
            Plat::Any,
            "HTTP 429. rhythr makes one request per uncached map, so this is the server's own \
             limit. A high count on this code is worth looking at: it means something asked more \
             than once, which the terms this feature exists under do not allow.",
        )],
    },
    Entry {
        id: "RH-NET-503",
        area: Area::Network,
        title: "rhythia.com answered with an error",
        fix: "Try again later, or load the map file by hand.",
        patterns: &["is unavailable right now", "status code 5", "server error", "status code 4"],
        causes: &[(
            Plat::Any,
            "A 5xx is the server having trouble and nothing on either side to fix. A 404 usually \
             means the map id no longer exists; a 403 means the request was blocked (a Cloudflare \
             challenge, a school block page) rather than the map being missing.",
        )],
    },
    Entry {
        id: "RH-NET-504",
        area: Area::Network,
        title: "the download did not finish",
        fix: "Press Download once more, or fetch the .sspm in a browser and load it.",
        patterns: &["timed out", "timeout"],
        causes: &[(
            Plat::Any,
            "The 60 s deadline is an OVERALL one and covers reading the body, so a 40 to 50 MB \
             map needs roughly 7 Mbit/s sustained or it is cut mid-body every time. A browser has \
             no such deadline and can resume, which is why that detour works.",
        )],
    },
    Entry {
        id: "RH-NET-505",
        area: Area::Network,
        title: "the secure connection could not be established",
        fix: "Turn off HTTPS scanning for rhythr, and check the system clock.",
        patterns: &["certificate", "tls", "handshake", "invalid peer"],
        causes: &[(
            Plat::Any,
            "rhythr uses its own compiled-in root list and does NOT read the system's, so \
             installing a CA does not help and neither does update-ca-certificates. Anything that \
             re-signs HTTPS (antivirus scanning, a company MITM proxy) fails here by design. A \
             wrong clock produces the same message.",
        )],
    },
    Entry {
        id: "RH-NET-506",
        area: Area::Network,
        title: "the answer was not a map",
        fix: "On public Wi-Fi finish the portal login first; otherwise the map is gone or private.",
        patterns: &["no beatmapfile", "bad response", "does not parse", "hash mismatch"],
        causes: &[(
            Plat::Any,
            "A 2xx carrying something unusable: a captive portal login page, a challenge page, or \
             a map with no file. \"No beatmapFile\" means the map was deleted, unlisted or made \
             private. If it happens for EVERY replay it is an API change, and that goes back to \
             the Rhythia team before anything here is changed.",
        )],
    },
    Entry {
        id: "RH-NET-507",
        area: Area::Network,
        title: "a map download is already running",
        fix: "Wait for the one in progress; it will load the map when it finishes.",
        patterns: &["download is already running"],
        causes: &[(
            Plat::Any,
            "Deliberate, not a fault. The Download button, the automatic fetch and a gesture that \
             drops several replays at once could otherwise each start their own request, and \
             several requests at once is bulk fetching, which the terms this feature exists under \
             forbid.",
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
            "Nothing in the NET list matched. The raw message names the URL and the stage. \
             Remember that rhythr talks to exactly one host, so anything about a different one is \
             a proxy or a block page.",
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
            "no preview yet",
        ],
        causes: &[(
            Plat::Any,
            "A command that needs a replay ran without one. Reachable by keyboard, by the Analyze \
             window, and by turning on HUD editing before the first preview frame has landed.",
        )],
    },
    Entry {
        id: "RH-APP-602",
        area: Area::App,
        title: "a render is already running",
        fix: "Wait for it to finish, or cancel it first.",
        patterns: &["rendering in progress", "already rendering", "already running"],
        causes: &[(
            Plat::Any,
            "Deliberate: benchmarks, preview renders and exports all refuse while a render holds \
             the GPU, because sharing it makes both slower and the timings meaningless.",
        )],
    },
    Entry {
        id: "RH-APP-603",
        area: Area::App,
        title: "the renderer crashed",
        fix: "Save diagnostics and report it; this one is always a bug.",
        patterns: &["renderer crashed", "panicked", "engine crashed"],
        causes: &[(
            Plat::Any,
            "A panic on a render thread, caught so the app survives. Read the text in the \
             parentheses rather than the sentence around it: \"Wait timed out\", \"device lost\" \
             or \"surface\" mean the GPU went away mid-render (RH-GPU-204), a shader name means \
             the driver rejected a pipeline (RH-GPU-205).",
        )],
    },
    Entry {
        id: "RH-APP-604",
        area: Area::App,
        title: "the settings could not be read or written",
        fix: "Look for settings.json.broken next to settings.json: the old settings are in it.",
        patterns: &["could not save settings", "settings file", "config dir", "settings.json"],
        causes: &[
            (
                Plat::Windows,
                "%APPDATA%\\rhythr is not writable: Controlled folder access, an antivirus or \
                 backup agent holding the file open, a roaming profile that is offline or over \
                 quota, or a full system drive. A leftover settings.json.tmp is the trace this \
                 leaves.",
            ),
            (
                Plat::Linux,
                "~/.config/rhythr owned by root because rhythr was started once with sudo, which \
                 is the single most common cause. Also a full or read-only home, or HOME and \
                 XDG_CONFIG_HOME both unset (a systemd unit, a bare su), which sends settings \
                 into the working directory.",
            ),
            (
                Plat::MacOs,
                "~/Library/Application Support is not writable, which sandboxing or a migrated \
                 account can cause.",
            ),
        ],
    },
    Entry {
        id: "RH-APP-605",
        area: Area::App,
        title: "the window itself could not start",
        fix: "Windows: install the Edge WebView2 runtime. Linux: install webkit2gtk-4.1.",
        patterns: &["webview", "webkit", "building tauri"],
        causes: &[
            (
                Plat::Windows,
                "The WebView2 runtime is missing or blocked by policy. The installer normally \
                 fetches it, so this is offline installs and stripped LTSC or Server images. The \
                 app has no console in release builds, so the failure is silent: nothing opens.",
            ),
            (
                Plat::Linux,
                "webkit2gtk-4.1 or libsoup3 missing, no DISPLAY or WAYLAND_DISPLAY (a plain SSH \
                 session), or a blank window from WebKitGTK's DMA-BUF renderer. rhythr disables \
                 that renderer itself, so a blank window usually means someone set \
                 WEBKIT_DISABLE_DMABUF_RENDERER=0. Start it from a terminal to see the error.",
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
             only lead. A code that shows up here often is a missing row.",
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
                "RH-FFM-105",
            ),
            (
                "video export failed: ffmpeg never connected to the frame socket",
                "RH-FFM-106",
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

    /// Broad patterns are useful and dangerous in the same way, so the pairs
    /// that could steal each other's messages are pinned here. Order in
    /// CODES is what decides them, and reordering the list is exactly the
    /// kind of edit that would break this silently.
    #[test]
    fn the_broad_patterns_do_not_steal_each_others_messages() {
        // "wait timed out" is a lost GPU, not a slow download.
        assert_eq!(
            classify("renderer crashed: wgpu error: The requested Wait timed out").id,
            "RH-GPU-204"
        );
        // "already running" belongs to the render guard, unless it is the
        // download guard, which says so.
        assert_eq!(
            classify("a map download is already running").id,
            "RH-NET-507"
        );
        assert_eq!(classify("rendering in progress").id, "RH-APP-602");
        // A refused write is a folder problem wherever the words came from.
        assert_eq!(
            classify("Error opening output /home/x/out.mp4: Permission denied").id,
            "RH-OUT-301"
        );
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

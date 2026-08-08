# rhythr error codes

Generated from `crates/errcode/src/lib.rs`. Do not edit by hand: run
`RHYTHR_UPDATE_DOCS=1 cargo test -p rhythia-errcode` after changing the table.

A code looks like `RH-FFM-101-L`. The last letter is the system it came from
(`W`indows, `L`inux, `M`acOS), because the same failure usually has a different
cause on each. Codes are stable: a row is never renumbered or reused, and a
failure that stops existing keeps its row marked as retired.

Users find these codes in the app next to the error, and at the top of the file
written by Settings > Diagnostics.

## Encoding (ffmpeg) (FFM)

### RH-FFM-101 ffmpeg could not be started

Shown to the user: Point rhythr at an ffmpeg binary under Advanced, or install one.

- **Windows**: The installer puts ffmpeg.exe next to the app and the PATH is not consulted, so this means that copy is gone: deleted, quarantined by antivirus (static GPL builds are a frequent false positive), blocked by SmartScreen or AppLocker (os error 5), or the app folder was copied without its resources.
- **Linux**: The distro package is missing: the AppImage carries its own copy, but deb and rpm only depend on one, so a dpkg -i without dependencies or a Fedora install without RPM Fusion leaves nothing to run. A dangling symlink or a non-executable stub on PATH shadows the bundled copy and reads the same way.
- **macOS**: Not installed (brew install ffmpeg), or Gatekeeper quarantined a hand-placed binary.
- Recognised by: `could not start ffmpeg`, `ffmpeg not found`, `ffmpeg does not run`, `ffmpeg could not be run`

### RH-FFM-102 the chosen encoder does not exist in this ffmpeg

Shown to the user: Set the encoder to Auto, which only offers encoders that passed a real test.

- **all systems**: A build without that encoder compiled in. Auto probes with a real test encode and falls back, so a code here means the encoder was forced (the CLI does not probe a forced choice) or the ffmpeg path changed after the probe, which is cached per path for the life of the process.
- **Linux**: Fedora and RHEL ship ffmpeg-free, and openSUSE a patent-free build, with no libx264 and no nvenc, and the rpm's dependency is satisfied by it. Enable RPM Fusion and install the full ffmpeg, or use the AppImage.
- **Windows**: Only after the ffmpeg path was repointed at a minimal or LGPL build. The bundled one always carries libx264, nvenc, qsv and amf.
- Recognised by: `unknown encoder`, `encoder not found`, `could not find encoder`, `unrecognized option`

### RH-FFM-103 the hardware encoder refused the job

Shown to the user: Switch the encoder to x264, or update the graphics driver and restart rhythr.

- **Windows**: The driver is older than the encoder API this ffmpeg was built against, the iGPU is disabled in the BIOS (QSV), AMF is missing because the driver came from Windows Update rather than Adrenalin, or every NVENC session is taken (consumer drivers cap them and a running stream or recording holds one). All of them also fail inside RDP.
- **Linux**: VAAPI needs a readable /dev/dri render node, so a user outside the render and video groups gets exactly this; it also needs the libva driver (mesa-va-drivers, intel-media-va-driver). NVENC needs the proprietary driver plus libnvidia-encode, which nouveau does not have. AMD does not ship AMF on Linux at all, so an AMF line in the diagnostics is expected there.
- **macOS**: NVENC, QSV and AMF do not exist here. Only VideoToolbox and x264 do.
- Recognised by: `cannot load nvcuda`, `cannot load libcuda`, `no capable devices found`, `openencodesessionex failed`, `no device available`, `device creation failed`, `failed to initialise vaapi`, `no va display`, `no working vaapi`, `amf failed`

### RH-FFM-104 the encoder cannot handle this frame size

Shown to the user: Render at 4K or below, or choose the x264 software encoder.

- **all systems**: Every consumer H.264 hardware encoder tops out at 4096x4096, on all three systems and every vendor. The availability probe encodes 256x256, so it cannot see this coming. 8K is a software-encoder job.
- Recognised by: `does not support encoding at size`, `hardware does not support`, `width not divisible`, `not divisible by 2`

### RH-FFM-105 ffmpeg stopped in the middle of the render

Shown to the user: Check free space, then try again with x264; save diagnostics if it repeats.

- **all systems**: ffmpeg died while frames were still coming. Its own last words are appended to the message and are worth more than the errno in front of them. A status 1 within a second of starting is not a full disk: run the resolved ffmpeg with -version by hand.
- **Linux**: Signal 9 with an empty stderr is the OOM killer, which 4K reaches on 8 GB machines. Signal 25 is a file size limit: a FAT32 or exFAT output disk stops at 4 GB, and ulimit -f does the same.
- **Windows**: Antivirus or Controlled folder access terminating ffmpeg.exe mid-encode, a driver reset taking the encoder session with it, the output drive disconnecting, or the machine sleeping. Signal names do not exist here, so the message ends with a bare status number.
- Recognised by: `writing frame`, `broken pipe`, `ffmpeg exited`, `exited with status`, `stopped by signal`

### RH-FFM-106 the frame socket never came up

Shown to the user: Turn off "send frames over a local connection" under Advanced; the pipe always works.

- **Windows**: A security suite intercepting loopback connections (Windows Firewall itself does not filter loopback, which is why the listener binds 127.0.0.1 explicitly). Some let the small probe through and block the real connection.
- **Linux**: Rare: an ffmpeg built with --disable-network, a Flatpak or Snap ffmpeg without network permission, an AppArmor or SELinux policy, or a rule on lo.
- **macOS**: The local network prompt was denied for rhythr.
- Recognised by: `never connected to the frame socket`, `could not arm the frame socket`, `could not settle the frame socket`, `frame socket failed`

### RH-FFM-107 every frame was accepted and no file came out

Shown to the user: Free about one more video's worth of space and render again; a local disk beats a share.

- **all systems**: The encode reached the end and the muxer failed at the last step. The output is written with +faststart, which rewrites the finished file once, so a disk with room for exactly one copy runs out during the rewrite. A network share dropping, or an x264 preset from a hand-edited settings file, do the same.
- Recognised by: `nothing was written into output file`, `error while filtering`

### RH-FFM-100 the encoder failed

Shown to the user: Save diagnostics and report it; ffmpeg's own output is in the file.

- **all systems**: Nothing in the FFM list matched. The message carries ffmpeg's first diagnostic line and its last two, which is the whole diagnosis; if the same text turns up twice, give it a row.

## Graphics (GPU)

### RH-GPU-201 no usable graphics adapter

Shown to the user: Update the graphics driver. WGPU_BACKEND=gl forces the OpenGL backend.

- **Windows**: No DX12 and no Vulkan adapter was enumerated: only the Microsoft Basic Display Adapter is installed, the driver predates feature level 11_0, or the session has no GPU (RDP, a service context, a VM without paravirtualisation).
- **Linux**: The common one, and usually not the user's fault: no Vulkan ICD is installed (mesa-vulkan-drivers, vulkan-radeon, vulkan-intel, nvidia-utils), and none of the packages depend on a driver ICD. Also /dev/dri unreadable without the render and video groups, a headless or SSH session, or a sandbox without the GPU socket.
- **macOS**: Effectively impossible on supported hardware: every Metal-capable Mac works.
- Recognised by: `no compatible gpu adapter`, `no usable gpu`, `no graphics adapter`, `no usable gpu adapter`

### RH-GPU-202 the GPU was found but would not open a device

Shown to the user: Reboot once, then update the graphics driver.

- **all systems**: An adapter answered and then failed to hand out a device at wgpu's default limits (8192 textures, 128 MiB storage bindings). A driver left wedged by an earlier GPU reset is the usual cause, and a reboot is the usual fix.
- **Linux**: Often the OpenGL fallback: wgpu falls back to GL when no Vulkan ICD is present, and old Mesa GL reports limits below the defaults. Installing a real ICD is the fix, not a setting. Also a kernel and Mesa mismatch after a partial upgrade, which only a reboot clears.
- Recognised by: `gpu device request failed`, `would not open a device`, `requestdevice`

### RH-GPU-203 the GPU ran out of memory

Shown to the user: Drop one resolution step and close other GPU-heavy programs.

- **all systems**: A renderer at 8K wants roughly 0.8 to 1 GB before any skin or background: three readback buffers of about 127 MB each, three NV12 buffers, plus the targets. The preview renderer keeps its own device alive during an export and a live Analyze session is a third. RHYTHR_NO_GPU_NV12=1 frees part of it, slower.
- **Windows**: Windows pages GPU memory to system RAM instead of failing, so a low-VRAM card more often crawls than crashes. When it does fail it says device removed.
- **Linux**: Most drivers do not page out: the allocation fails outright, or the process is killed by the OOM killer when the driver backs the buffers with system RAM.
- Recognised by: `out of memory`, `outofmemory`, `not enough memory`, `allocation failed`

### RH-GPU-204 the GPU was lost part way through the render

Shown to the user: Re-run at a lower resolution or frame rate, then update the graphics driver.

- **Windows**: The watchdog resets the GPU when a submission does not return in about two seconds, which a 4K frame on a mid-range or laptop card can reach. Also a vendor driver updating itself during a render, or an overclock.
- **Linux**: A kernel-level GPU hang or reset (amdgpu "GPU reset begin", i915 "GPU HANG", an nvidia Xid), a driver module reload, or eviction pressure from another GPU program. dmesg right after the failure names the real one.
- Recognised by: `device lost`, `device_removed`, `device removed`, `wait timed out`, `surface lost`, `surface validation`

### RH-GPU-205 the driver rejected one of rhythr's shaders

Shown to the user: Update the driver. RHYTHR_NO_GPU_NV12=1 removes the compute shader meanwhile.

- **Windows**: A driver whose shader compiler rejects a pipeline (older Intel integrated drivers are the usual suspects), or a broken driver install.
- **Linux**: A Mesa or RADV version with a SPIR-V regression, or the GL fallback backend, where the NV12 compute shader needs GLES 3.1-class compute that an older stack does not provide.
- Recognised by: `shader module`, `shader validation`, `spir-v`, `dxil`, `pipeline creation`

### RH-GPU-200 the graphics layer failed

Shown to the user: Save diagnostics and report it; it names the GPU, the backend and the driver.

- **all systems**: Nothing in the GPU list matched. Read the gpu line of the diagnostics file first: a device type of Cpu, or a name like llvmpipe, lavapipe or Microsoft Basic Render Driver, means the render was on the processor and the complaint is really about speed.

## Writing the file (OUT)

### RH-OUT-303 the finished render could not be moved into place

Shown to the user: The .rhythr-part.mp4 file IS the video: close whatever holds the target open, or rename it.

- **Windows**: The dominant case: the destination is open in a media player, Explorer's preview or thumbnail handler, a sync client or an antivirus scan, and Windows cannot replace an open file. A read-only attribute on the existing file does it too. The encode itself was fine.
- **all systems**: Elsewhere this is nearly unreachable, since rename replaces a file others have open. It needs the destination name to be a directory, an immutable file, a full or read-only filesystem, or a mount without atomic replace.
- Recognised by: `could not be moved into place`, `being used by another process`, `sharing violation`

### RH-OUT-301 not allowed to write there

Shown to the user: Try the home folder first: that says whether it is the folder or the app.

- **Windows**: Defender's Controlled folder access protects Videos, Documents and Desktop by default, and Videos is the default output folder, so a stock install can hit this with nothing done wrong. Allow-listing rhythr.exe alone does NOT help: ffmpeg is a separate process and needs its own entry. Also a read-only share, a sync folder over quota, or Program Files without elevation.
- **Linux**: A folder owned by root because it was created once with sudo, a read-only mount (an NTFS or exFAT partition auto-mounted ro after an unclean Windows shutdown), a gvfs or MTP mount, or Flatpak confinement, where any path outside the granted portals looks exactly like this.
- **macOS**: Access to Desktop, Documents or Downloads has not been granted yet, which macOS gates behind a prompt that can be dismissed.
- Recognised by: `permission denied`, `access is denied`, `os error 13`, `os error 5`

### RH-OUT-302 the disk is full

Shown to the user: Free about twice the expected video size, or pick a folder on a bigger drive.

- **all systems**: A full-length 1080p60 render is several GB, and the file is rewritten once at the end (+faststart), so the peak is about two copies. Look in the output folder for abandoned .rhythr-part.mp4 files as well: each is a full-length partial encode, nothing ever sweeps them, and a hard kill or a power cut leaves one behind.
- Recognised by: `no space left`, `not enough space`, `os error 28`, `os error 112`, `quota`

### RH-OUT-304 the output path is not usable

Shown to the user: Type a shorter file name, or render to a shallow folder.

- **Windows**: The 260-character path limit. A deep sync folder plus an auto-derived "Player - Song (1.02-1.34).mp4" can leave the final name fitting and the .rhythr-part sibling (12 characters longer) not, which is why the failure can look like a folder that plainly exists. ffmpeg is not long-path aware, so enabling long paths does not rescue it. Forbidden characters and reserved names like CON land here too.
- **Linux**: The other half of the same problem: the name is capped at 150 CHARACTERS, and a Japanese, Korean or Cyrillic title is 2 to 3 bytes per character, so it can pass the cap and still exceed the 255-BYTE limit of ext4, xfs and btrfs.
- **macOS**: A colon in the name: Finder shows it as a slash and the API rejects it.
- Recognised by: `no output folder set`, `invalid argument`, `filename, directory name, or volume label`, `os error 123`, `file name too long`, `os error 36`

### RH-OUT-300 the file could not be written

Shown to the user: Save into the home folder and try again.

- **all systems**: Nothing in the OUT list matched. The output folder is in the diagnostics file (redacted). Note the Copy button next to Save diagnostics, which produces the same report without touching the disk: useful when this is the failure being reported.

## Replays and maps (MAP)

### RH-MAP-401 the replay file is damaged, truncated, or not a replay

Shown to the user: Re-export the replay from the game and copy the whole file.

- **all systems**: There is no signature check on .rhr, so a file that is not a replay at all (a map, a skin export, the game's config.json, a renamed PNG) produces the same byte-level complaint as a truncated one. A healthy .rhr is a header plus a multiple of 17 bytes.
- **Windows**: Explorer hides known extensions, so run.rhr.txt looks like run.rhr, and drag-and-drop accepts anything the Browse dialog would filter out. A copy interrupted mid-write, an unhydrated cloud placeholder, or the game still holding the file open all give a short file.
- **Linux**: Usually a copy made out of the Proton prefix while the game was still writing it: there is no mandatory locking, so a mid-write read always succeeds partially and always lands here.
- Recognised by: `unexpected end of data`, `malformed varint`, `is not valid utf-8`, `is not a valid length`

### RH-MAP-402 the map file could not be read

Shown to the user: Load the .sspm or .rhm itself, or let rhythr download the map.

- **all systems**: Only .sspm, .rhm and the game's cache .json are accepted, so a downloaded .zip fails on its extension. "Could not find EOCD" means the archive is incomplete (interrupted download); "unsupported version 3" means the map is newer than this build; "missing or invalid field Notes" is what the game's own config.json produces, because a dropped .json is tried as a map before it is tried as a skin.
- Recognised by: `sspm:`, `.rhm archive`, `map json:`, `unsupported map file extension`, `-byte limit`, `eocd`

### RH-MAP-403 the loaded map is not the one the replay was played on

Shown to the user: Let rhythr download the map by the replay's own id, or browse for the exact chart.

- **all systems**: Hash or map id disagree, or most recorded hits find no note. Rendering is not blocked and the video carries no warning, so the render can silently show the wrong notes. When neither heuristic fires, the same situation reads as "possibly manipulated", which accuses the player for somebody else's map file. A locally browsed map is not hash-checked at all.
- Recognised by: `may not match`, `does not match the replay`, `wrong map`

### RH-MAP-404 the game files could not be read

Shown to the user: Use Locate and pick the real game binary (about 280 MB), not a shortcut.

- **Windows**: Detect looks at the Steam registry key and the two Program Files defaults, so a non-Steam, itch or manual install is invisible. It picks the LARGEST matching binary, so a launcher stub or an old Sound Space Plus install can win. The extraction writes about 600 files and then swaps the folder, which fails while antivirus, the search indexer or a second rhythr holds one of them open.
- **Linux**: Detect knows five home paths only (.local/share/Steam, .steam/steam, .steam/root, the Flatpak and Snap paths), so a distro-package Steam with data elsewhere, a system-wide install or a library on a root-only NTFS mount is invisible, and there is no registry to fall back on. A failed swap means the data directory is full, read-only, or root-owned from an earlier sudo run.
- **macOS**: The bundle was moved, or its files are somewhere rhythr has not been granted access to.
- Recognised by: `game assets`, `could not find the game`, `.pck`, `no game resources`, `not found in any steam library`, `no usable skin assets`

### RH-MAP-405 the replay's timestamps are damaged

Shown to the user: Re-export the replay from the game, or clip a shorter range to render it anyway.

- **all systems**: A replay's length is the timestamp of its last frame, taken as read, and one damaged stamp is enough to make that days. It passes every integrity check (the header agrees with the frames), and the render it asks for is a progress bar that never moves and a file that grows until the disk is full, which is why it is refused before it starts.
- Recognised by: `which no run is`, `claims to be`

### RH-MAP-400 the replay or map could not be loaded

Shown to the user: Save diagnostics and report it, with the file if you can share it.

- **all systems**: Nothing in the MAP list matched. A replay recorded by a NEWER game version parses with today's field order and is then reported as inconsistent or broken, never as "rhythr is too old", so check the version in the report before believing any verdict.

## Downloads (NET)

### RH-NET-501 rhythia.com could not be reached

Shown to the user: Check the connection, then press Download again. A local map file always works.

- **all systems**: IMPORTANT: rhythr does not read the system proxy settings at all, so on a proxy-only school or company network every request fails here while the browser on the same machine works. There is no proxy setting in the app; the way through is to download the .sspm in a browser and load it with Browse.
- **Windows**: Also a firewall or security suite blocking the freshly installed unsigned rhythr.exe outbound, or plain DNS failure.
- **Linux**: Also systemd-resolved down, a broken /etc/resolv.conf, an egress firewall rule, or a sandbox without network.
- Recognised by: `dns failed`, `dns error`, `failed to lookup`, `no route to host`, `connection refused`, `network is unreachable`, `os error 11001`

### RH-NET-502 rhythia.com is rate-limiting us

Shown to the user: Wait a minute and press Download once. Repeated clicks make it worse.

- **all systems**: HTTP 429. rhythr makes one request per uncached map, so this is the server's own limit. A high count on this code is worth looking at: it means something asked more than once, which the terms this feature exists under do not allow.
- Recognised by: `rate-limiting`, `429`, `too many requests`

### RH-NET-503 rhythia.com answered with an error

Shown to the user: Try again later, or load the map file by hand.

- **all systems**: A 5xx is the server having trouble and nothing on either side to fix. A 404 usually means the map id no longer exists; a 403 means the request was blocked (a Cloudflare challenge, a school block page) rather than the map being missing.
- Recognised by: `is unavailable right now`, `status code 5`, `server error`, `status code 4`

### RH-NET-504 the download did not finish

Shown to the user: Press Download once more, or fetch the .sspm in a browser and load it.

- **all systems**: The 60 s deadline is an OVERALL one and covers reading the body, so a 40 to 50 MB map needs roughly 7 Mbit/s sustained or it is cut mid-body every time. A browser has no such deadline and can resume, which is why that detour works.
- Recognised by: `timed out`, `timeout`

### RH-NET-505 the secure connection could not be established

Shown to the user: Turn off HTTPS scanning for rhythr, and check the system clock.

- **all systems**: rhythr uses its own compiled-in root list and does NOT read the system's, so installing a CA does not help and neither does update-ca-certificates. Anything that re-signs HTTPS (antivirus scanning, a company MITM proxy) fails here by design. A wrong clock produces the same message.
- Recognised by: `certificate`, `tls`, `handshake`, `invalid peer`

### RH-NET-506 the answer was not a map

Shown to the user: On public Wi-Fi finish the portal login first; otherwise the map is gone or private.

- **all systems**: A 2xx carrying something unusable: a captive portal login page, a challenge page, or a map with no file. "No beatmapFile" means the map was deleted, unlisted or made private. If it happens for EVERY replay it is an API change, and that goes back to the Rhythia team before anything here is changed.
- Recognised by: `no beatmapfile`, `bad response`, `does not parse`, `hash mismatch`

### RH-NET-507 a map download is already running

Shown to the user: Wait for the one in progress; it will load the map when it finishes.

- **all systems**: Deliberate, not a fault. The Download button, the automatic fetch and a gesture that drops several replays at once could otherwise each start their own request, and several requests at once is bulk fetching, which the terms this feature exists under forbid.
- Recognised by: `download is already running`

### RH-NET-500 the map could not be downloaded

Shown to the user: Try again, or load the map file by hand.

- **all systems**: Nothing in the NET list matched. The raw message names the URL and the stage. Remember that rhythr talks to exactly one host, so anything about a different one is a proxy or a block page.

## Application (APP)

### RH-APP-601 nothing is loaded yet

Shown to the user: Load a replay first; the map follows from it.

- **all systems**: A command that needs a replay ran without one. Reachable by keyboard, by the Analyze window, and by turning on HUD editing before the first preview frame has landed.
- Recognised by: `no replay loaded`, `no map loaded`, `replay has no online map id`, `no preview yet`

### RH-APP-602 a render is already running

Shown to the user: Wait for it to finish, or cancel it first.

- **all systems**: Deliberate: benchmarks, preview renders and exports all refuse while a render holds the GPU, because sharing it makes both slower and the timings meaningless.
- Recognised by: `rendering in progress`, `already rendering`, `already running`

### RH-APP-603 the renderer crashed

Shown to the user: Save diagnostics and report it; this one is always a bug.

- **all systems**: A panic on a render thread, caught so the app survives. Read the text in the parentheses rather than the sentence around it: "Wait timed out", "device lost" or "surface" mean the GPU went away mid-render (RH-GPU-204), a shader name means the driver rejected a pipeline (RH-GPU-205).
- Recognised by: `renderer crashed`, `panicked`, `engine crashed`

### RH-APP-604 the settings could not be read or written

Shown to the user: Look for settings.json.broken next to settings.json: the old settings are in it.

- **Windows**: %APPDATA%\rhythr is not writable: Controlled folder access, an antivirus or backup agent holding the file open, a roaming profile that is offline or over quota, or a full system drive. A leftover settings.json.tmp is the trace this leaves.
- **Linux**: ~/.config/rhythr owned by root because rhythr was started once with sudo, which is the single most common cause. Also a full or read-only home, or HOME and XDG_CONFIG_HOME both unset (a systemd unit, a bare su), which sends settings into the working directory.
- **macOS**: ~/Library/Application Support is not writable, which sandboxing or a migrated account can cause.
- Recognised by: `could not save settings`, `settings file`, `config dir`, `settings.json`

### RH-APP-605 the window itself could not start

Shown to the user: Windows: install the Edge WebView2 runtime. Linux: install webkit2gtk-4.1.

- **Windows**: The WebView2 runtime is missing or blocked by policy. The installer normally fetches it, so this is offline installs and stripped LTSC or Server images. The app has no console in release builds, so the failure is silent: nothing opens.
- **Linux**: webkit2gtk-4.1 or libsoup3 missing, no DISPLAY or WAYLAND_DISPLAY (a plain SSH session), or a blank window from WebKitGTK's DMA-BUF renderer. rhythr disables that renderer itself, so a blank window usually means someone set WEBKIT_DISABLE_DMABUF_RENDERER=0. Start it from a terminal to see the error.
- Recognised by: `webview`, `webkit`, `building tauri`

### RH-APP-600 something went wrong

Shown to the user: Save diagnostics and report it.

- **all systems**: The catch-all. Nothing anywhere in the table matched, so the message itself is the only lead. A code that shows up here often is a missing row.

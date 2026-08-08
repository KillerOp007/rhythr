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

Shown to the user: Point rhythr at an ffmpeg binary in Settings, or install one.

- **Windows**: No ffmpeg.exe on PATH and none next to rhythr.exe. The installer does not ship one, so this is the expected first-run failure on a bare Windows box.
- **Linux**: The distro package is missing (the AppImage carries its own ffmpeg, the deb and rpm depend on the system one). Check that the binary in the setting is executable, and that a Flatpak or snap sandbox is not hiding it.
- **macOS**: Not installed (brew install ffmpeg), or Gatekeeper quarantined a hand-placed binary.
- Recognised by: `could not start ffmpeg`, `ffmpeg not found`, `ffmpeg does not run`

### RH-FFM-102 the chosen encoder does not exist in this ffmpeg

Shown to the user: Pick a different encoder in Settings; x264 works on every build.

- **all systems**: A build without the encoder compiled in. Distro builds routinely drop nvenc and qsv; the probe in video.rs is meant to catch this before a render, so a code here also means the probe and the render disagreed.
- **Linux**: Also seen when ffmpeg is a Flatpak or snap wrapper whose feature set differs from the one the probe measured.
- Recognised by: `unknown encoder`, `encoder not found`, `could not find encoder`, `unrecognized option`

### RH-FFM-103 the hardware encoder refused the job

Shown to the user: Switch the encoder to x264, or update the graphics driver.

- **Windows**: Driver too old for the ffmpeg build, or every NVENC session is taken (consumer drivers cap concurrent sessions, and a running stream or recording holds one).
- **Linux**: VA-API needs /dev/dri access: a user outside the video and render groups gets exactly this. NVENC additionally needs the proprietary driver, not nouveau.
- **macOS**: NVENC, QSV and AMF do not exist here; only VideoToolbox and x264 do.
- Recognised by: `cannot load nvcuda`, `no capable devices found`, `openencodesessionex failed`, `no device available`, `device creation failed`, `failed to initialise vaapi`, `no va display`, `amf failed`

### RH-FFM-104 ffmpeg stopped in the middle of the render

Shown to the user: Try again with x264; if it repeats, save diagnostics and report it.

- **all systems**: ffmpeg died while frames were still coming. Its own last words are appended to the message and say more than the errno: out of memory, an unsupported pixel format, or a full disk are the usual three.
- **Linux**: A kill by signal 9 with no stderr is the OOM killer, which at 4K is a real possibility on 8 GB machines.
- Recognised by: `writing frame`, `broken pipe`, `ffmpeg exited`, `exited with status`

### RH-FFM-105 the frame socket never came up

Shown to the user: Turn the socket transport off in Settings (the pipe always works).

- **Windows**: A firewall or endpoint-protection product blocking a loopback listener. It is loopback only, but that is not a distinction every product makes.
- **Linux**: Rare. A sandbox without loopback networking (Flatpak with no network permission) is the case that has been seen.
- **macOS**: The local network prompt was denied for rhythr.
- Recognised by: `never connected to the frame socket`, `could not arm the frame socket`, `could not settle the frame socket`, `frame socket failed`

### RH-FFM-100 the encoder failed

Shown to the user: Save diagnostics and report it; the ffmpeg output is in the file.

- **all systems**: Nothing in the FFM list matched. The raw ffmpeg text in the diagnostics file is the whole diagnosis; if it turns out to be a repeating case, give it its own code.

## Graphics (GPU)

### RH-GPU-201 no usable graphics adapter

Shown to the user: Update the graphics driver, or start rhythr with WGPU_BACKEND=gl.

- **Windows**: No DX12 and no Vulkan: a very old GPU, a machine running on the Microsoft Basic Display Adapter, or an RDP session with no GPU passed through.
- **Linux**: The usual one. No Vulkan ICD installed (mesa-vulkan-drivers or the vendor driver), a headless session with no render node, or a user without access to /dev/dri (groups video and render).
- **macOS**: Effectively impossible on supported hardware; every Metal-capable Mac is fine.
- Recognised by: `no compatible gpu adapter`, `no usable gpu`, `no graphics adapter`, `no usable gpu adapter`

### RH-GPU-202 the GPU was found but would not open a device

Shown to the user: Update the graphics driver, or start rhythr with WGPU_BACKEND=gl.

- **all systems**: An adapter answered and then failed to hand out a device: usually a driver that is out of date or has been left wedged by an earlier crash.
- **Linux**: Also a mismatch between a freshly updated kernel module and a still-loaded old one, which a reboot fixes and nothing else does.
- Recognised by: `gpu device request failed`, `would not open a device`, `requestdevice`

### RH-GPU-203 the GPU ran out of memory

Shown to the user: Render at a smaller resolution, or close other GPU-heavy programs.

- **all systems**: 4K with three readback slots is roughly 100 MB of VRAM on top of the scene. On a card that is already carrying a game or a browser with hardware acceleration, that is the frame that does not fit.
- **Windows**: A DXGI_ERROR_DEVICE_REMOVED here is usually TDR: the driver reset because a single submission took longer than two seconds.
- Recognised by: `out of memory`, `outofmemory`, `device_removed`, `allocation failed`

### RH-GPU-200 the graphics layer failed

Shown to the user: Save diagnostics and report it; it names the GPU and the driver.

- **all systems**: Nothing in the GPU list matched. The gpu section of the diagnostics file has the adapter, the backend and the driver version, which is normally enough.

## Writing the file (OUT)

### RH-OUT-303 the finished render could not be moved into place

Shown to the user: The video is safe under its .rhythr-part name; close whatever has the target open.

- **Windows**: The destination is open in a media player or being scanned by antivirus. Windows refuses the rename; the encode itself was fine.
- **all systems**: The part file and the destination are on different filesystems, or the destination folder disappeared during the render.
- Recognised by: `could not be moved into place`, `being used by another process`, `sharing violation`

### RH-OUT-301 not allowed to write there

Shown to the user: Pick a different output folder in Settings.

- **Windows**: Writing into Program Files, a drive root, or a OneDrive folder that is currently locked by sync.
- **Linux**: A folder owned by root, or a read-only mount. Inside a Flatpak sandbox, any path outside the granted portals looks exactly like this.
- **macOS**: The app has not been granted access to Desktop, Documents or Downloads yet, which macOS gates behind a prompt that can be dismissed.
- Recognised by: `permission denied`, `access is denied`, `os error 13`, `os error 5`

### RH-OUT-302 the disk is full

Shown to the user: Free some space or choose another drive, then render again.

- **all systems**: A 4K render at high quality is easily several GB, and the part file lives beside the final one, so the peak is one file, not two.
- Recognised by: `no space left`, `not enough space`, `os error 28`, `os error 112`

### RH-OUT-304 the output path is not usable

Shown to the user: Choose the output folder again and keep the file name simple.

- **Windows**: A character Windows forbids in a name (: * ? " < > |), a reserved name like CON, or a path over 260 characters on a system without long paths enabled. Map titles supply all three regularly.
- **Linux**: A name over 255 bytes, which a long song title in a non-Latin script reaches sooner than it looks.
- **macOS**: A colon in the name: HFS and APFS show it as a slash in Finder and reject it in the API.
- Recognised by: `no output folder set`, `invalid argument`, `filename, directory name, or volume label`, `os error 123`, `file name too long`, `os error 36`

### RH-OUT-300 the file could not be written

Shown to the user: Try a different output folder, then save diagnostics if it repeats.

- **all systems**: Nothing in the OUT list matched. The output folder is listed (redacted) in the diagnostics file.

## Replays and maps (MAP)

### RH-MAP-401 the replay file is damaged or truncated

Shown to the user: Re-export the replay from the game and load it again.

- **all systems**: The parser ran off the end of the file. A copy that was interrupted, a file pulled out of a partially synced folder, or a replay the game itself never finished writing.
- Recognised by: `unexpected end of data`, `malformed varint`, `is not valid utf-8`, `is not a valid length`

### RH-MAP-402 the map file could not be read

Shown to the user: Load the map again, or let rhythr download it from rhythia.com.

- **all systems**: A map format rhythr does not know, a damaged archive, or an .sspm from a newer version of the format than this build handles.
- Recognised by: `sspm:`, `.rhm archive`, `map json:`, `unsupported map file extension`, `-byte limit`

### RH-MAP-403 the loaded map is not the one the replay was played on

Shown to the user: Let rhythr download the right map, or load the exact file you played.

- **all systems**: Hash or map id disagree, or the hit pattern does not line up. This is a warning, not a failure: the render still runs, and it is the reason a run can look 'manipulated' when the only mistake was the chart.
- Recognised by: `may not match`, `does not match the replay`, `wrong map`

### RH-MAP-404 the game files could not be read

Shown to the user: Point rhythr at the game folder again in Settings.

- **Windows**: The game was moved or reinstalled and the stored path is stale.
- **Linux**: A Steam or Proton install whose real path sits under a compatdata folder that changes; also a Flatpak game whose files are outside rhythr's sandbox.
- **macOS**: The .app bundle was moved to the Bin, or the files are inside a bundle rhythr has not been granted access to.
- Recognised by: `game assets`, `could not find the game`, `.pck`

### RH-MAP-400 the replay or map could not be loaded

Shown to the user: Save diagnostics and report it with the file if you can share it.

- **all systems**: Nothing in the MAP list matched. The loaded section of the diagnostics file names the file (without the folder) and what was parsed out of it.

## Downloads (NET)

### RH-NET-501 rhythia.com could not be reached

Shown to the user: Check the internet connection, then press Download again.

- **all systems**: No connection, or DNS is not answering. rhythr only ever contacts rhythia.com, so a working browser and a failing download usually means a proxy.
- **Windows**: A corporate proxy or a VPN that captures DNS. ureq does not read the system proxy settings, so a machine that only reaches the internet through one fails here while every browser on it works.
- **Linux**: A sandbox without network access (Flatpak), or a resolver that only exists inside the host namespace.
- Recognised by: `dns failed`, `dns error`, `failed to lookup`, `no route to host`, `connection refused`, `network is unreachable`, `os error 11001`

### RH-NET-502 rhythia.com is rate-limiting us

Shown to the user: Wait a moment and press Download again.

- **all systems**: HTTP 429. rhythr makes one request per uncached map, so this is the server's own limit being hit, not a loop on our side. If a code shows a high count, look for a retry that is not backing off.
- Recognised by: `rate-limiting`, `429`, `too many requests`

### RH-NET-503 rhythia.com is having trouble

Shown to the user: Try again later, or load the map file by hand.

- **all systems**: An HTTP 5xx from the API. Nothing on the user's side to fix, and nothing on ours either.
- Recognised by: `is unavailable right now`, `status code 5`, `server error`

### RH-NET-504 the download timed out

Shown to the user: Try again; if it keeps timing out, download the map in a browser.

- **all systems**: 10 s to connect, 60 s overall. A large map on a slow line can genuinely exceed the second one.
- Recognised by: `timed out`, `timeout`

### RH-NET-505 the secure connection could not be established

Shown to the user: Check the system clock and any antivirus that inspects HTTPS.

- **Windows**: Antivirus doing HTTPS inspection with its own root, which rhythr's bundled root store does not contain. A wrong system clock produces the same message.
- **Linux**: A machine with no CA bundle at all, which happens in minimal containers.
- **macOS**: Usually the clock, occasionally a corporate MDM root.
- Recognised by: `certificate`, `tls`, `handshake`, `invalid peer`

### RH-NET-506 the map on the server is not the one the replay used

Shown to the user: The map was re-uploaded; load the file you played if you still have it.

- **all systems**: The API answered but with something unusable: a map with no file, or a file that does not parse. A re-upload under the same id is the common case, which is what fetchedFor in the cache metadata exists to remember.
- Recognised by: `no beatmapfile`, `does not parse`, `hash mismatch`

### RH-NET-500 the map could not be downloaded

Shown to the user: Try again, or load the map file by hand.

- **all systems**: Nothing in the NET list matched. The raw message carries the ureq text, which names the URL and the stage.

## Application (APP)

### RH-APP-601 nothing is loaded yet

Shown to the user: Load a replay first; the map follows from it.

- **all systems**: A command that needs a replay ran without one. Reachable by keyboard shortcuts and by the Analyze window before anything is loaded.
- Recognised by: `no replay loaded`, `no map loaded`, `replay has no online map id`

### RH-APP-602 a render is already running

Shown to the user: Wait for it to finish, or cancel it first.

- **all systems**: Deliberate: benchmarks, preview renders and exports all refuse while a render holds the GPU, because sharing it makes both slower and the timings meaningless.
- Recognised by: `rendering in progress`, `already rendering`

### RH-APP-603 the renderer crashed

Shown to the user: Save diagnostics and report it; this one is always a bug.

- **all systems**: A panic on the render thread, caught so the app survives. The diagnostics file has the GPU, the settings and the last render, which is what makes it reproducible.
- Recognised by: `renderer crashed`, `panicked`

### RH-APP-604 the settings file could not be read or written

Shown to the user: Settings will fall back to defaults; check the config folder's permissions.

- **Windows**: %APPDATA%\rhythr is on a roaming profile that is not available, or is being held by another copy of rhythr.
- **Linux**: $XDG_CONFIG_HOME points somewhere unwritable, or $HOME is not set at all (which happens when rhythr is launched from a service unit).
- **macOS**: ~/Library/Application Support is not writable, which sandboxing or a migrated user account can cause.
- Recognised by: `could not save settings`, `settings file`, `config dir`

### RH-APP-600 something went wrong

Shown to the user: Save diagnostics and report it.

- **all systems**: The catch-all. Nothing anywhere in the table matched, so the message itself is the only lead. A code that shows up often here is a missing row.

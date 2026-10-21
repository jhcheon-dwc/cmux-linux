# cmux-linux — WSL2 한글 환경용 포크 (jhcheon-dwc)

[bradwilson331/cmux-linux](https://github.com/bradwilson331/cmux-linux)를 Windows의 WSL2(WSLg)에서 한글로 매일 쓸 수 있게 고친 포크입니다. **설치와 사용은 `ime-support` 브랜치 기준입니다.** 원본 README는 [아래](#upstream-readme)에 그대로 있습니다.

## 왜 포크했나

원본 0.1.0을 WSL2에 설치하면 기본적인 터미널 사용부터 막혔습니다.

- 한글 입력이 안 됨 (입력기 연결 코드가 없음)
- 복사/붙여넣기(`Ctrl+Shift+C`/`V`)가 동작하지 않음
- cmux 안의 Claude Code에서 `Ctrl+V` 이미지 붙여넣기가 안 됨
- Claude Code 알림이 오지 않음
- 창(pane)을 클릭해도 포커스가 옮겨가지 않음, 스크롤 방향이 반대
- `Ctrl+Shift+X`로 창을 닫으면 앱 전체가 크래시
- `1 | 2 | 3`에서 가운데 창을 닫으면 그 창이 멈춘 채로 남음

또 원본 `main`은 ghostty 서브모듈이 존재하지 않는 커밋을 가리켜 빌드가 안 됩니다([#1](https://github.com/bradwilson331/cmux-linux/issues/1), [#3](https://github.com/bradwilson331/cmux-linux/issues/3)). 그래서 빌드를 고친 [PR #4](https://github.com/bradwilson331/cmux-linux/pull/4)(peteonrails) 위에서 작업했습니다.

## 원본과 다른 점

`ime-support` 브랜치 = 원본 `main` + PR #4 + 아래 커밋.

| 문제 | 원인 | 커밋 |
|---|---|---|
| 한글 등 IME 입력 불가 | 키 입력이 입력기(`GtkIMContext`)를 거치지 않음. Ghostty GTK와 같은 방식으로 preedit/commit 연결 | `6b9f1962` |
| 복사/붙여넣기 안 됨 | 앱 단축키가 대상 터미널을 잘못 찾음(워크스페이스 간 `pane_id` 중복), 클립보드 콜백 ABI가 Ghostty와 불일치 | `21a33345` |
| `Ctrl+글자`가 앱에 안 감 (Claude Code `Ctrl+V` 이미지 등) | 키 이벤트의 `unshifted_codepoint`를 0으로 보내서 kitty 키보드 프로토콜 인코딩 실패 | `37510a52` |
| 알림이 안 옴 | OSC 9/777 알림(Claude Code가 사용)을 무시함. `notify-send`로 전달하도록 추가 | `c36e923e` |
| 클릭으로 포커스 이동 불가 | 클릭 시 활성 창이 바뀌지 않아 포커스가 이전 창으로 되돌아감 | `af59e492` |
| 스크롤 방향 반대 | Ghostty GTK와 달리 스크롤 값 부호를 뒤집지 않음 | `093e78e0` |
| 창 닫기 시 크래시 | 터미널을 해제한 뒤 포커스 해제 이벤트가 해제된 터미널에 접근 | `9e8436de` |
| 가운데 창이 멈춘 채 남음 | 중첩 분할을 접을 때 위젯을 기존 부모에서 떼지 않아 GTK가 교체를 거부 | `2cf95066` |

원본 저장소에는 IME 지원을 [이슈 #5](https://github.com/bradwilson331/cmux-linux/issues/5)와 [PR #6](https://github.com/bradwilson331/cmux-linux/pull/6)(`feat/ime-support` 브랜치, IME 커밋만)으로 올려 두었습니다.

이 외에 WSLg 환경 설정은 코드가 아니라 [`contrib/wsl/`](contrib/wsl/)의 파일과 설치 스크립트로 맞춥니다.

## 설치 (WSL2)

확인한 환경: Windows 11 + WSL2 Ubuntu 26.04 (WSLg), sudo 사용 가능.

```bash
git clone -b ime-support https://github.com/jhcheon-dwc/cmux-linux.git ~/projects/cmux-linux
cd ~/projects/cmux-linux
contrib/wsl/setup.sh
```

처음 빌드는 Zig 다운로드와 libghostty 빌드 때문에 몇 분 걸립니다. 끝나면 새 터미널을 열고 `cmux-app`을 실행하거나 Windows 시작 메뉴에서 cmux를 엽니다.

`setup.sh`가 하는 일 (여러 번 실행해도 됩니다):

1. 패키지 설치: Rust, GTK4 개발 패키지, Go, `ibus`/`ibus-hangul`, 한글·심볼·이모지 폰트, `wl-clipboard` 등
2. 빌드: 서브모듈 → `scripts/setup-linux.sh`(Zig 0.15.2 + libghostty) → `cargo build` → `cmuxd-remote`(Go) → `.deb` 생성 후 `dpkg -i`
3. 사용자 설정 설치 (아래 표)
4. 시작 메뉴 항목이 실행 래퍼를 쓰도록 수정
5. Windows: AutoHotkey v2 설치(winget), 입력 보정 스크립트(한/영키, 휠 가속)를 시작프로그램에 등록하고 실행
6. Windows: 알림 클릭 처리기 설치, `cmux-focus:` URI를 현재 사용자(HKCU)에 등록

옵션: `--skip-build`(설정만 다시 적용), `--no-windows`(Windows 쪽 작업 생략)

### 설치되는 파일

| 위치 | 원본 | 역할 |
|---|---|---|
| `~/.local/bin/cmux-app` | `contrib/wsl/bin/cmux-app` | 실행 래퍼. 한글 입력 환경변수, `GALLIUM_DRIVER=d3d12`(WSLg 기본 GL이 실패함), ibus 데몬과 한글 엔진 준비 |
| `~/.local/bin/notify-send` | `contrib/wsl/bin/notify-send` | WSLg엔 알림 서버가 없어서 `powershell.exe`로 Windows 토스트를 띄움 |
| `~/.config/cmux-wsl/ime.sh` | `contrib/wsl/shell/ime.sh` | `~/.zshrc`/`~/.bashrc`에서 불러옴. ibus 환경변수, 데몬 시작, 한글 엔진 지정 |
| `~/.config/gtk-{3,4}.0/settings.ini` | `contrib/wsl/config/gtk/settings.ini` | GTK 다크 테마 (없을 때만 설치) |
| `~/.config/ghostty/config.ghostty` | `contrib/wsl/config/ghostty/config.ghostty` | 폰트(D2Coding) 등. cmux와 Ghostty가 같이 씀 (없을 때만 설치) |
| `%USERPROFILE%\wslg-input.ahk` | `contrib/wsl/windows/wslg-input.ahk` | WSLg 창(`msrdc.exe`)에서만 동작. 한/영키를 `Shift+Space`로 바꿈(WSLg가 한/영키를 전달하지 않음). 마우스 휠만 2배로 가속(터치패드는 그대로) |
| `%LOCALAPPDATA%\cmux\cmux-focus.ps1` | `contrib/wsl/windows/cmux-focus.ps1` | 알림 클릭 처리기. `HKCU\Software\Classes\cmux-focus` URI로 등록되어, 클릭한 알림의 워크스페이스를 선택하고 cmux 창을 앞으로 가져옴 |

## 알림

Claude Code 등이 보내는 OSC 9/777 알림, 터미널 벨, `cmux notify` 명령이 같은 경로로 처리됩니다.

- 알림을 보낸 창(pane)과 워크스페이스에 사이드바 점(attention)이 표시됩니다.
- 그 창을 보고 있을 때(cmux 창이 활성이고 그 창에 포커스)는 알림을 띄우지 않습니다. 벨은 기존처럼 cmux 창이 비활성일 때만 알림을 띄웁니다.
- Windows 알림을 클릭하면 해당 워크스페이스로 이동하고 cmux 창이 앞으로 옵니다.
- 스크립트에서 보내기: `cmux notify --title "빌드" --body "끝났습니다" [--surface <UUID>] [--workspace <UUID>]` (생략하면 `$CMUX_SURFACE_ID`/`$CMUX_WORKSPACE_ID`, 없으면 현재 워크스페이스). 보낸 창을 지정하지 않으면(`--surface` 없음) 보고 있어도 알림을 띄웁니다. 소켓 메서드는 `notification.create`.

`~/.config/cmux/config.toml`:

```toml
[notifications]
enabled = true                # 데스크톱 알림(토스트) 표시. 꺼도 사이드바 점은 표시됨
suppress_when_focused = true  # 보고 있는 창에서 온 OSC/cmux notify 알림은 무시
bell = true                   # cmux 창이 비활성일 때 벨 알림 표시
# 알림마다 실행할 명령 (sh -c, 기다리지 않음). 환경변수:
# CMUX_NOTIFICATION_TITLE, CMUX_NOTIFICATION_BODY, CMUX_NOTIFICATION_KIND(bell|osc|cli),
# CMUX_WORKSPACE_ID, CMUX_WORKSPACE_NAME, CMUX_SURFACE_ID
command = 'echo "$CMUX_NOTIFICATION_TITLE" >> ~/cmux-notifications.log'
```

## 사용 시 주의

- **cmux는 래퍼로 실행하세요.** 터미널의 `cmux-app` 또는 시작 메뉴로 실행하면 됩니다. `/usr/bin/cmux-app`을 직접 실행하면 한글 입력이 안 됩니다.
- **한/영 전환은 `Shift+Space` 또는 한/영키입니다.** 한글 엔진(`ibus engine` → `hangul`) 안에서만 동작합니다. 안 되면 `ibus engine hangul`을 실행하세요.
- **`.deb`를 다시 설치하면 시작 메뉴 항목이 원래대로 돌아갑니다.** `contrib/wsl/setup.sh --skip-build`로 다시 맞추세요.
- **Claude Code 세션 안에서 cmux를 띄우지 마세요.** Claude 환경변수(`CLAUDE_CODE_CHILD_SESSION` 등)를 물려받아, cmux 안의 Claude Code가 대화 기록을 저장하지 않습니다.
- **Windows 알림의 발신자는 "Windows PowerShell"로 표시됩니다.** 알림이 안 보이면 Windows 알림 설정에서 PowerShell을 허용하세요.
- **PowerToys로 한/영키를 리매핑하지 마세요.** 앱 단위 지정이 안 돼서 Windows 전체의 한/영 전환이 막힙니다. AutoHotkey 스크립트를 쓰세요.
- **스크롤 속도는 Ghostty 설정이 아니라 `wslg-input.ahk`의 `WheelFactor`로 조절하세요.** WSLg에서는 Ghostty의 `mouse-scroll-multiplier`로 휠과 터치패드를 따로 맞추기 어렵습니다(휠 스크롤의 첫 이벤트만 정밀 스크롤로 처리되어 첫 칸만 튐). Windows에서는 휠(120의 배수)과 터치패드(그 외 값)가 구분되므로, 스크립트가 휠만 가속합니다. 값을 바꾼 뒤 `contrib/wsl/setup.sh --skip-build`로 다시 설치하세요.
- 패키지 버전은 원본과 같은 `0.1.0`이라 `dpkg` 정보로는 원본과 구분되지 않습니다.

## 단축키

실제 기본값입니다(`src/config.rs`). 아래 원본 README의 단축키 표는 실제와 다릅니다. 메뉴와 도움말 창(햄버거 메뉴 → Keyboard Shortcuts)에는 설정을 반영한 현재 단축키가 표시됩니다.

| 단축키 | 동작 | `[shortcuts]` 키 |
|---|---|---|
| `Ctrl+N` | 새 워크스페이스 (현재 터미널의 작업 디렉터리에서 시작) | `new_workspace` |
| `Ctrl+Shift+W` | 워크스페이스 닫기 | `close_workspace` |
| `Ctrl+]` / `Ctrl+[` | 다음 / 이전 워크스페이스 | `next_workspace` / `prev_workspace` |
| `Ctrl+Shift+R` | 워크스페이스 이름 바꾸기 | `rename_workspace` |
| `Ctrl+1`~`9` | N번째 워크스페이스로 이동 | `workspace_1`~`workspace_9` |
| `Ctrl+B` | 사이드바 보이기/숨기기 | `toggle_sidebar` |
| `Ctrl+D` | 오른쪽으로 분할 (현재 창의 작업 디렉터리에서 시작) | `split_right` |
| `Ctrl+Shift+D` | 아래로 분할 (현재 창의 작업 디렉터리에서 시작) | `split_down` |
| `Ctrl+Shift+X` | 창(pane) 닫기 | `close_pane` |
| `Ctrl+Shift+←/→/↑/↓` | 이전 / 다음 창으로 포커스 (트리 순서 기준) | `focus_left` / `focus_right` / `focus_up` / `focus_down` |
| `Ctrl+Alt+1`~`9` | N번째 창으로 포커스 (왼쪽→오른쪽, 위→아래 순서) | `focus_pane_1`~`focus_pane_9` |
| ``Ctrl+Alt+` `` | 직전에 포커스했던 창으로 돌아가기 | `focus_last_pane` |
| `Ctrl+Alt+Shift+←/→/↑/↓` | 현재 창에 가장 가까운 분할선을 그 방향으로 5% 이동 | `resize_left` / `resize_right` / `resize_up` / `resize_down` |
| `Ctrl+Alt+=` | 모든 분할을 같은 크기로 | `equalize_splits` |
| `Ctrl+Shift+S` | SSH 워크스페이스 | `new_ssh_workspace` |
| `Ctrl+Shift+B` / `Ctrl+Shift+Q` | 브라우저 미리보기 열기 / 닫기 | `browser_open` / `browser_close` |
| `Ctrl+Shift+C` / `Ctrl+Shift+V` | 복사 / 붙여넣기 | (변경 불가) |
| `Ctrl+Q` | 종료 | (변경 불가) |

바꾸려면 `~/.config/cmux/config.toml`에 GTK 가속기 형식으로 적고 cmux를 다시 시작합니다. 잘못된 값은 경고 후 기본값을 씁니다.

```toml
[shortcuts]
focus_last_pane = "<Ctrl><Alt>Tab"
resize_left = "<Ctrl><Alt><Shift>h"
equalize_splits = "<Ctrl><Alt>0"
```

- 새 창/워크스페이스의 작업 디렉터리는 셸이 OSC 7로 알린 경로를 먼저 쓰고, 없으면 원래 창 셸의 현재 디렉터리(`/proc`)를 씁니다. 이를 위해 cmux가 띄운 셸에는 `CMUX_SURFACE_ID`(창 UUID) 환경변수가 붙습니다.
- GNOME 등 일부 데스크톱은 `Ctrl+Alt+Shift+방향키`를 작업 공간 이동에 씁니다. 그런 환경에서는 위처럼 다른 키로 바꾸세요.
- 같은 동작은 소켓 CLI로도 할 수 있습니다: `cmux focus-pane-number 2`, `cmux last-pane`, `cmux resize-pane left --amount 0.1`, `cmux equalize-panes`.

## 알려진 제한

- 가로 탭이 없습니다. 왼쪽 사이드바의 워크스페이스가 탭 역할을 합니다(`Ctrl+N` 새로 만들기, `Ctrl+1`~`9` 이동).
- 기본 단축키 `Ctrl+N`, `Ctrl+B`, `Ctrl+D`, `Ctrl+1`~`9`가 터미널 앱의 키와 겹칩니다. `~/.config/cmux/`의 `[shortcuts]`에서 바꿀 수 있습니다.
- 알림을 클릭해도 cmux 창으로 이동하지 않습니다.
- 소켓 CLI의 `send-text`/`read-text`는 이 환경에서 동작하지 않았습니다.
- 실행 시 GTK CSS 경고(`max-width` 등)가 나오지만 동작에는 영향이 없습니다.

---

<a id="upstream-readme"></a>

<h1 align="center">cmux for Linux</h1>
<p align="center">A GPU-accelerated terminal multiplexer with tabs, splits, workspaces, browser automation, and socket CLI control — powered by Ghostty</p>

<p align="center">
  <img src="./docs/assets/main-first-image.png" alt="cmux screenshot" width="900" />
</p>

## About

cmux for Linux is a full native port of [cmux](https://github.com/manaflow-ai/cmux) (originally a macOS Swift/AppKit app) rebuilt in Rust on GTK4. It provides the same experience — tabs, splits, workspaces, notifications, browser automation, and a scriptable socket API — running natively on Linux with GPU-accelerated terminal rendering via Ghostty.

Built for developers running multiple AI coding agents (Claude Code, Codex, etc.) in parallel who need visibility into which agent needs attention and the ability to script browser interactions alongside terminal sessions.

## Features

- **GPU-accelerated terminal** — Powered by libghostty with GTK4 GtkGLArea rendering
- **Workspaces, tabs, and split panes** — Organize parallel agent sessions
- **Notification system** — Per-pane bell tracking, sidebar indicators, desktop notifications
- **In-app browser** — CDP-based browser automation with accessibility tree snapshots, element interaction, and JS evaluation via [agent-browser](https://github.com/vercel-labs/agent-browser)
- **Scriptable CLI** — `cmux` CLI with 34+ subcommands for workspaces, panes, surfaces, and browser control
- **Socket API** — v2 JSON-RPC over Unix socket with SO_PEERCRED auth
- **SSH remote workspaces** — cmuxd-remote deployment with bidirectional PTY proxy and reconnect
- **Ghostty compatible** — Reads your existing `~/.config/ghostty/config` for themes, fonts, and colors
- **Session persistence** — Atomic save/restore of full split tree topology with divider ratios

## Install

### Debian / Ubuntu (.deb)

```bash
sudo dpkg -i cmux_0.1.0_amd64.deb
sudo apt-get install -f  # install dependencies if needed
```

### Fedora / RHEL (.rpm)

```bash
sudo rpm -i cmux-0.1.0-1.x86_64.rpm
```

### Build from source

```bash
# Prerequisites: Rust toolchain, GTK4 dev libraries.
# Zig 0.15.2 is required to build libghostty; setup-linux.sh downloads it
# automatically if the system Zig is a different version.
git submodule update --init --recursive
./scripts/setup-linux.sh    # install GTK4 headers if missing, build libghostty.a
cargo build --release --workspace
```

This produces `target/release/cmux`, `target/release/cmux-app`, and
`target/release/agent-browser`. Without `--workspace`, cargo builds only the
`cmux` / `cmux-app` binaries and skips `agent-browser`, which the browser
preview pane and the `.deb` / `.rpm` packages both expect.

## Browser Automation

Agents running inside cmux can discover and use browser automation via the `cmux browser` CLI:

```bash
# Open a site (https:// auto-prepended if no scheme)
cmux browser open slashdot.org            # returns surface:1 handle

# Interact with the page
cmux browser surface:1 snapshot --interactive  # accessibility tree with element refs
cmux browser surface:1 click e3               # click element by ref
cmux browser surface:1 fill e5 "search term"  # fill input field
cmux browser surface:1 eval 'document.title'  # evaluate JavaScript

# Navigation
cmux browser surface:1 goto example.com
cmux browser surface:1 back
cmux browser surface:1 forward
cmux browser surface:1 reload

# Management
cmux browser list                          # list browser surfaces
cmux browser close --surface surface:1     # close a surface
```

Browser commands default to JSON output (agents are the primary consumers). Use `--no-json` for human-readable output.

## CLI Reference

```bash
cmux --help                    # all commands
cmux browser --help            # browser subcommands

# Terminal management
cmux list-workspaces           # list all workspaces
cmux new-workspace             # create workspace
cmux list-surfaces             # list terminal surfaces
cmux split --direction horizontal  # split current pane
cmux list-panes                # list all panes

# System
cmux identify                  # instance info (version, platform, pid)
cmux ping                      # check connectivity
cmux raw <method> --params '{}' # send arbitrary JSON-RPC
```

### Socket Path

The cmux socket is at `$XDG_RUNTIME_DIR/cmux/cmux.sock` (typically `/run/user/$UID/cmux/cmux.sock`).

Override with `CMUX_SOCKET` environment variable or `--socket` flag.

## Agent Skills

When installed via .deb or .rpm, agent skills are available at `/usr/share/cmux/skills/`:

- **cmux** — Core terminal multiplexer skill (workspaces, panes, surfaces, socket CLI)
- **cmux-browser** — Browser automation skill (open sites, interact with pages, extract data)

A `CLAUDE.md` at `/usr/share/cmux/CLAUDE.md` references skill paths so Claude Code discovers them automatically.

## Architecture

- **Language:** Rust
- **UI toolkit:** GTK4 via gtk4-rs
- **Terminal engine:** Ghostty (manaflow-ai fork) via libghostty C FFI
- **Async runtime:** tokio + glib spawn_local bridge
- **Browser automation:** agent-browser daemon with CDP protocol
- **Remote sessions:** Go daemon (cmuxd-remote) reused from macOS codebase
- **Socket protocol:** v2 JSON-RPC, wire-compatible with macOS cmux

## Input Methods (IME)

Terminal panes support GTK input methods, so you can type Korean, Japanese, Chinese, and dead-key text through ibus or fcitx. In-progress (preedit) text is drawn at the cursor, and the candidate window opens next to it.

Most desktop sessions set this up already. If composition doesn't start, make sure the GTK input module is selected before launching cmux:

```bash
# ibus (e.g. ibus-hangul, ibus-mozc)
export GTK_IM_MODULE=ibus XMODIFIERS=@im=ibus
# fcitx5
export GTK_IM_MODULE=fcitx XMODIFIERS=@im=fcitx
```

The input method daemon must be running and have an engine selected (for ibus, `ibus engine` prints the active engine).

## Keyboard Shortcuts

> **Fork note:** this table does not match the actual defaults. See the [단축키](#단축키) table in the fork section at the top.

| Shortcut | Action |
|----------|--------|
| Ctrl+Shift+T | New workspace |
| Ctrl+1–8 | Jump to workspace 1–8 |
| Ctrl+Shift+D | Split right |
| Ctrl+D | Split down |
| Ctrl+Shift+W | Close workspace |
| Ctrl+W | Close pane |
| Ctrl+Shift+] / [ | Next / previous workspace |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous pane |
| Ctrl+Shift+F | Find |
| Ctrl+Shift+K | Clear scrollback |

Shortcuts are configurable via TOML config file.

## Building Packages

```bash
# Build all release binaries
cargo build --release --bin cmux --bin cmux-app
cargo build --release -p agent-browser

# Build .deb
./packaging/scripts/build-deb.sh

# Build .rpm
./packaging/scripts/build-rpm.sh

# Validate packages
./packaging/scripts/validate-deb.sh
./packaging/scripts/validate-rpm.sh
```

## License

This project is licensed under the GNU Affero General Public License v3.0 or later (`AGPL-3.0-or-later`).

See `LICENSE` for the full text.

## Upstream

Linux port of [cmux](https://github.com/manaflow-ai/cmux) by [manaflow-ai](https://github.com/manaflow-ai).

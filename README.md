# cmux-linux — WSL2 한글 환경용 포크 (jhcheon-dwc)

[bradwilson331/cmux-linux](https://github.com/bradwilson331/cmux-linux)를 **Windows의 WSL2에서 한글 입력과 Claude Code로 매일 쓸 수 있게** 고친 포크입니다. 설치와 사용은 `ime-support` 브랜치 기준이고, 원본 README는 [아래](#upstream-readme)에 그대로 있습니다.

이런 분께 맞춥니다:

- Windows PC에서 WSL2(Ubuntu)로 개발하고
- 터미널에서 한글을 쓰고 (`Shift+Space` 또는 한/영키로 전환)
- Claude Code 같은 에이전트를 여러 개 띄워 두고, 어느 것이 답을 기다리는지 한눈에 보고 싶은 분

## 목차

- [빠른 설치](#빠른-설치)
- [설치 확인](#설치-확인)
- [업데이트와 제거](#업데이트와-제거)
- [문제 해결](#문제-해결)
- [단축키](#단축키)
- [탭](#탭) · [Claude Code 상태 표시](#claude-code-상태-표시) · [알림](#알림) · [브라우저 미리보기](#브라우저-미리보기)
- [원본과 다른 점](#원본과-다른-점) · [알려진 제한](#알려진-제한)

## 빠른 설치

### 0. 준비 (Windows)

- **Windows 11** (또는 Microsoft Store판 WSL이 설치된 Windows 10 22H2). WSL의 리눅스 GUI 앱 기능(WSLg)을 씁니다.
- PowerShell에서 WSL과 Ubuntu를 준비합니다. 이미 쓰고 있다면 `wsl --update`만 하면 됩니다.

  ```powershell
  wsl --install -d Ubuntu     # 처음이면. 설치 후 사용자 이름/비밀번호 설정
  wsl --update                # WSLg 최신화
  wsl --shutdown              # 업데이트 후 WSL 재시작
  ```

- Ubuntu **24.04 이상**이 필요합니다(GTK 4.14 이상). 확인한 환경은 Windows 11 + Ubuntu 26.04입니다.

### 1. 설치 (WSL의 Ubuntu 터미널에서)

```bash
git clone -b ime-support https://github.com/jhcheon-dwc/cmux-linux.git ~/projects/cmux-linux
cd ~/projects/cmux-linux
contrib/wsl/setup.sh --claude-hooks
```

- `sudo` 비밀번호를 물어봅니다. 처음 빌드는 Zig 다운로드와 libghostty 빌드 때문에 10분 넘게 걸릴 수 있습니다.
- `--claude-hooks`는 Claude Code를 쓰는 경우에만 붙입니다. 사이드바에 Claude Code 상태를 표시하는 훅을 `~/.claude/settings.json`에 추가합니다(기존 파일은 백업, 기존 설정은 유지).
- 브라우저 미리보기(`Ctrl+Shift+B`)도 쓰려면 `--browser`를 함께 붙입니다(Chrome을 내려받음).
- 여러 번 실행해도 안전합니다. 중간에 실패하면 원인을 고친 뒤 같은 명령을 다시 실행하세요.

### 2. 실행

1. **새 Ubuntu 터미널을 엽니다** (한글 입력기 설정이 셸에 적용됨).
2. `cmux-app`을 실행하거나, Windows 시작 메뉴에서 **cmux**를 엽니다.
3. Claude Code를 쓴다면 cmux 안에서 `claude`를 실행합니다. 아직 없다면 cmux 안에서 설치하세요: `curl -fsSL https://claude.ai/install.sh | bash`

> `/usr/bin/cmux-app`을 직접 실행하면 한글 입력이 안 됩니다. 항상 `cmux-app`(래퍼) 또는 시작 메뉴로 실행하세요.

### setup.sh가 하는 일

1. 패키지 설치: 빌드 도구, GTK4, Go, `ibus-hangul`, 한글·심볼·이모지 글꼴, `wl-clipboard` 등. 배포판 Rust가 1.88보다 오래됐으면 rustup으로 `~/.cargo`에 최신 Rust 설치, D2Coding 글꼴이 없으면 `~/.local/share/fonts`에 설치
2. 빌드: 서브모듈 → Zig 0.15.2 + libghostty → `cargo build` → `cmuxd-remote`(Go) → `.deb` 생성 후 설치
3. 사용자 설정 설치 (아래 표). `~/.zshrc`/`~/.bashrc`에 한글 입력기 설정 한 줄 추가
4. 시작 메뉴 항목이 실행 래퍼를 쓰도록 수정
5. Windows: AutoHotkey v2 설치(winget), 입력 보정 스크립트(한/영키, 휠 가속)를 시작프로그램에 등록하고 실행
6. Windows: 알림 클릭 처리기 설치, `cmux-focus:` URI를 현재 사용자(HKCU)에 등록

옵션: `--skip-build`(빌드 없이 설정만 다시 적용), `--no-windows`(Windows 쪽 작업 생략), `--claude-hooks`, `--browser`

| 설치 위치 | 원본 | 역할 |
|---|---|---|
| `~/.local/bin/cmux-app` | `contrib/wsl/bin/cmux-app` | 실행 래퍼. 한글 입력 환경변수, `GALLIUM_DRIVER=d3d12`(WSLg 기본 GL이 실패함), ibus 데몬과 한글 엔진 준비 |
| `~/.local/bin/notify-send` | `contrib/wsl/bin/notify-send` | WSLg에는 알림 서버가 없어서 `powershell.exe`로 Windows 알림을 띄움 |
| `~/.config/cmux-wsl/ime.sh` | `contrib/wsl/shell/ime.sh` | 셸 시작 시 ibus 환경변수, 데몬 시작, 한글 엔진 지정 |
| `~/.config/gtk-{3,4}.0/settings.ini` | `contrib/wsl/config/gtk/settings.ini` | GTK 다크 테마 (없을 때만 설치) |
| `~/.config/ghostty/config.ghostty` | `contrib/wsl/config/ghostty/config.ghostty` | 글꼴(D2Coding) 등. cmux와 Ghostty가 같이 씀 (없을 때만 설치) |
| `%USERPROFILE%\wslg-input.ahk` | `contrib/wsl/windows/wslg-input.ahk` | WSLg 창(`msrdc.exe`)에서만 동작. 한/영키를 `Shift+Space`로 바꿈(WSLg가 한/영키를 전달하지 않음). 마우스 휠만 3배로 가속(터치패드는 그대로) |
| `%LOCALAPPDATA%\cmux\cmux-focus.ps1` | `contrib/wsl/windows/cmux-focus.ps1` | 알림 클릭 처리기. 클릭한 알림의 워크스페이스를 선택하고 cmux 창을 앞으로 가져옴 |

## 설치 확인

cmux를 열고 차례로 해 보세요.

| 확인 | 방법 | 기대 결과 |
|---|---|---|
| 한글 입력 | `Shift+Space`(또는 한/영키) 후 `gksrmf` 입력 | `한글` |
| 복사/붙여넣기 | 텍스트를 드래그 → `Ctrl+Shift+C` → `Ctrl+Shift+V` | 그대로 붙음 |
| 분할/탭 | `Ctrl+D`, `Ctrl+Shift+T` | 오른쪽 분할, 새 탭 |
| 알림 | `cmux notify --title 테스트 --body 알림` | Windows 오른쪽 아래에 알림. 클릭하면 cmux로 이동 |
| Claude Code 상태 | cmux 안에서 `claude` 실행 후 질문 | 사이드바 워크스페이스에 **Running** 표시와 진행 막대, 끝나면 **Idle** |

## 업데이트와 제거

업데이트:

```bash
cd ~/projects/cmux-linux
git pull
contrib/wsl/setup.sh        # 처음에 --claude-hooks/--browser를 썼다면 이번엔 생략해도 됨
```

그다음 cmux를 완전히 껐다가 다시 켭니다. 워크스페이스, 분할, 탭, 각 창의 작업 디렉터리는 세션에 저장돼 다시 열 때 복원됩니다.

제거:

```bash
sudo apt remove cmux
rm -f ~/.local/bin/cmux-app ~/.local/bin/notify-send ~/.local/share/applications/com.cmux_lx.terminal.desktop
rm -rf ~/.config/cmux-wsl
sed -i '/# cmux-linux WSL/d' ~/.zshrc ~/.bashrc 2>/dev/null
reg.exe delete 'HKCU\Software\Classes\cmux-focus' /f
```

Windows 시작프로그램의 `wslg-input.lnk`(`Win+R` → `shell:startup`)와 `%USERPROFILE%\wslg-input.ahk`도 지우면 됩니다. Claude Code 훅은 `~/.claude/settings.json`에서 `cmux claude-hook`이 들어간 항목을 지웁니다.

## 문제 해결

| 증상 | 해결 |
|---|---|
| 한글이 안 써짐 | 래퍼(`cmux-app` 또는 시작 메뉴)로 실행했는지 확인. `ibus engine`이 `hangul`이 아니면 `ibus engine hangul` 실행 후 cmux 재시작 |
| 한/영키가 안 됨 | `Shift+Space`는 되는지 확인. 한/영키는 Windows의 AutoHotkey 스크립트가 바꿔 줍니다. 작업 표시줄에 AutoHotkey 아이콘이 없으면 `%USERPROFILE%\wslg-input.ahk`를 더블클릭 |
| 창이 안 뜨거나 검은 화면 | Windows에서 `wsl --update` → `wsl --shutdown` 후 다시 시도. 래퍼 없이 실행하면 GL이 실패할 수 있음 |
| 시작 메뉴로 열면 한글이 안 됨 | `.deb`를 다시 설치하면 시작 메뉴 항목이 원래대로 돌아갑니다. `contrib/wsl/setup.sh --skip-build` |
| 알림이 안 뜸 | 발신자가 **Windows PowerShell**로 표시됩니다. Windows 설정 → 알림에서 PowerShell을 허용하고, 방해 금지 모드를 확인 |
| Claude Code 상태가 안 바뀜 | `--claude-hooks`로 설치했는지, 그 뒤 **새로 시작한** Claude Code 세션인지 확인. cmux 안의 셸에서 `echo $CMUX_SURFACE_ID`가 비어 있지 않아야 함 |
| cmux 안의 Claude Code가 대화 기록을 저장하지 않음 | Claude Code 세션 안에서 cmux를 띄우면 Claude 환경변수를 물려받습니다. cmux는 일반 터미널이나 시작 메뉴에서 실행 |
| 갑자기 키 입력이 안 됨 | `Ctrl+Shift+방향키`로 다른 창에 갔다 오면 풀립니다. 재현되면 `CMUX_DEBUG_INPUT=1 cmux-app 2> ~/cmux-input.log`로 실행해 로그와 함께 이슈를 남겨 주세요 |
| 브라우저 미리보기에 "No browser preview"만 보임 | Chrome이 없습니다. `contrib/wsl/setup.sh --skip-build --browser` |
| 스크롤이 너무 느리거나 빠름 | Ghostty 설정이 아니라 `contrib/wsl/windows/wslg-input.ahk`의 `WheelFactor`를 바꾼 뒤 `contrib/wsl/setup.sh --skip-build` |

참고:

- **PowerToys로 한/영키를 리매핑하지 마세요.** 앱 단위 지정이 안 돼서 Windows 전체의 한/영 전환이 막힙니다.
- WSLg에서는 Ghostty의 `mouse-scroll-multiplier`로 휠과 터치패드를 따로 맞추기 어렵습니다. Windows에서는 휠(120의 배수)과 터치패드(그 외 값)가 구분되므로, AutoHotkey 스크립트가 휠만 가속합니다.
- 패키지 버전은 원본과 같은 `0.1.0`이라 `dpkg` 정보로는 원본과 구분되지 않습니다.

## 단축키

실제 기본값입니다(`src/config.rs`). 메뉴와 도움말 창(햄버거 메뉴 → Keyboard Shortcuts)에는 설정을 반영한 현재 단축키가 표시됩니다. 아래 원본 README의 단축키 표는 실제와 다릅니다.

| 단축키 | 동작 | `[shortcuts]` 키 |
|---|---|---|
| `Ctrl+N` | 새 워크스페이스 (현재 터미널의 작업 디렉터리에서 시작) | `new_workspace` |
| `Ctrl+Shift+W` | 워크스페이스 닫기 | `close_workspace` |
| `Ctrl+]` / `Ctrl+[` | 다음 / 이전 워크스페이스 | `next_workspace` / `prev_workspace` |
| `Ctrl+Shift+]` / `Ctrl+Shift+[` | 다음 / 이전 워크스페이스 | `next_workspace_alt` / `prev_workspace_alt` |
| `Ctrl+Shift+R` | 워크스페이스 이름 바꾸기 | `rename_workspace` |
| `Ctrl+1`~`9` | N번째 워크스페이스로 이동 | `workspace_1`~`workspace_9` |
| `Ctrl+B` | 사이드바 보이기/숨기기 | `toggle_sidebar` |
| `Ctrl+D` | 오른쪽으로 분할 (현재 창의 작업 디렉터리에서 시작) | `split_right` |
| `Ctrl+Shift+D` | 아래로 분할 (현재 창의 작업 디렉터리에서 시작) | `split_down` |
| `Ctrl+Shift+X` | 현재 탭 닫기 (마지막 탭이면 창(pane) 닫기) | `close_pane` |
| `Ctrl+Shift+T` | 새 탭 (현재 터미널의 작업 디렉터리에서 시작) | `new_tab` |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | 다음 / 이전 탭 | `next_tab` / `prev_tab` |
| `Ctrl+PageDown` / `Ctrl+PageUp` | 다음 / 이전 탭 | `next_tab_alt` / `prev_tab_alt` |
| `Alt+1`~`9` | 현재 창의 N번째 탭으로 이동 | `goto_tab_1`~`goto_tab_9` |
| `Ctrl+Alt+R` | 현재 탭 이름 바꾸기 (macOS의 `Cmd+R`; `Ctrl+R`은 셸 기록 검색이라 피함) | `rename_tab` |
| `Ctrl+Shift+←/→/↑/↓` | 이전 / 다음 창으로 포커스 (트리 순서 기준) | `focus_left` / `focus_right` / `focus_up` / `focus_down` |
| `Ctrl+Alt+1`~`9` | N번째 창으로 포커스 (왼쪽→오른쪽, 위→아래 순서) | `focus_pane_1`~`focus_pane_9` |
| ``Ctrl+Alt+` `` | 직전에 포커스했던 창으로 돌아가기 | `focus_last_pane` |
| `Ctrl+Alt+Shift+←/→/↑/↓` | 현재 창에 가장 가까운 분할선을 그 방향으로 5% 이동 | `resize_left` / `resize_right` / `resize_up` / `resize_down` |
| `Ctrl+Alt+=` | 모든 분할을 같은 크기로 | `equalize_splits` |
| `Ctrl+Shift+Enter` | 현재 창을 워크스페이스 전체로 확대 / 되돌리기 (tmux의 zoom) | `toggle_pane_zoom` |
| `Ctrl+Shift+F` | 찾기 바 (`Enter`/`Shift+Enter`로 다음/이전, `Esc`로 닫기) | (변경 불가) |
| `Ctrl+Shift+S` | SSH 워크스페이스 | `new_ssh_workspace` |
| `Ctrl+Shift+B` / `Ctrl+Shift+Q` | 브라우저 미리보기 열기 / 닫기 | `browser_open` / `browser_close` |
| `Ctrl+Shift+C` / `Ctrl+Shift+V` | 복사 / 붙여넣기 | (변경 불가) |
| `Ctrl+Q` | 종료 | (변경 불가) |

바꾸려면 `~/.config/cmux/config.toml`에 GTK 가속기 형식으로 적고 cmux를 다시 시작합니다. 잘못된 값은 경고 후 기본값을 씁니다.

```toml
[shortcuts]
prev_workspace = "<Ctrl><Alt>bracketleft"   # Ctrl+[ 를 터미널의 Esc로 돌려주기
focus_last_pane = "<Ctrl><Alt>Tab"
equalize_splits = "<Ctrl><Alt>0"
```

- `Ctrl+[`는 터미널에서 `Esc`와 같은 키입니다. vim 등에서 `Ctrl+[`를 `Esc`로 쓴다면 위처럼 `prev_workspace`를 바꾸고 `Ctrl+Shift+[`를 쓰세요.
- 새 창/워크스페이스의 작업 디렉터리는 셸이 OSC 7로 알린 경로를 먼저 쓰고, 없으면 원래 창 셸의 현재 디렉터리(`/proc`)를 씁니다.
- 창 확대(`Ctrl+Shift+Enter`)는 다른 창을 숨길 뿐 닫지 않습니다. 확대된 창은 점선 테두리로 표시되고, 분할을 새로 만들거나 창을 닫거나 다른 창으로 포커스를 옮기면(tmux처럼) 원래 배치로 돌아갑니다. 확대 상태는 워크스페이스마다 따로 유지되며 세션에는 저장되지 않습니다.
- 같은 동작은 소켓 CLI로도 할 수 있습니다: `cmux focus-pane-number 2`, `cmux last-pane`, `cmux resize-pane left --amount 0.1`, `cmux equalize-panes`, `cmux zoom-pane [--on|--off]`.

## 탭

macOS cmux처럼 창(pane)마다 여러 터미널 탭을 열 수 있습니다. 탭이 2개 이상이면 창 위에 탭 막대가 나타납니다.

- `Ctrl+Shift+T`로 현재 터미널의 작업 디렉터리에서 새 탭을 엽니다(현재 탭 오른쪽에 추가).
- 탭에는 터미널 제목(OSC 0/2)이 표시되고, 없으면 `Terminal N`입니다. 백그라운드 탭에서 벨이나 알림이 오면 주황색 점이 붙습니다.
- 탭 클릭: 선택, 가운데 클릭 또는 `×`: 닫기, 드래그: 같은 창 안에서 순서 바꾸기.
- 탭 이름 바꾸기: 탭을 더블클릭하거나, 오른쪽 클릭 → **Rename Tab…**, 또는 `Ctrl+Alt+R`. 이름을 비우면 터미널 제목으로 돌아갑니다. 이름은 세션에 저장되고, 이름을 붙인 탭은 창에 탭이 하나뿐이어도 탭 막대에 표시됩니다. CLI: `cmux rename-tab "이름" [--surface UUID]`.
- 탭이 하나여도 탭 막대를 보려면 `~/.config/cmux/config.toml`에 `[ui]` `always_show_tabs = true`를 넣습니다.
- CLI: `cmux new-tab`, `cmux close-tab [UUID]`, `cmux select-tab <번호|UUID>`, `cmux next-tab`, `cmux prev-tab`, `cmux list-tabs [--pane UUID]`. 소켓 메서드는 `tab.new`/`tab.close`/`tab.select`/`tab.next`/`tab.prev`/`tab.list`/`tab.move`/`tab.rename`입니다.

## Claude Code 상태 표시

사이드바의 워크스페이스마다 에이전트 상태(**Running**, **Needs input**, **Idle**, **Error**)와 짧은 메시지를 보여 줍니다. Claude Code가 권한 확인이나 `AskUserQuestion`/`ExitPlanMode`에서 기다리면 **Needs input**이 됩니다. 응답하는 동안(Running)에는 행 아래에 움직이는 진행 막대가 나오고, 프로그램이 OSC 9;4로 실제 진행률을 보내면 그 값을 표시합니다.

[설치](#1-설치-wsl의-ubuntu-터미널에서)할 때 `--claude-hooks`를 붙이면 됩니다. 나중에 훅만 추가하려면:

```bash
contrib/wsl/setup.sh --skip-build --no-windows --claude-hooks
```

[`contrib/claude/hooks.json`](contrib/claude/hooks.json)의 훅을 `~/.claude/settings.json`에 합칩니다. 기존 파일은 `settings.json.bak-<시각>`으로 백업하고, 이미 등록된 훅은 다시 넣지 않습니다. 등록 후 새로 시작한 Claude Code 세션부터 적용됩니다. 훅 명령은 `cmux claude-hook <이벤트> 2>/dev/null || true` 형태라서, cmux 밖에서 실행하거나 cmux가 꺼져 있어도 Claude Code를 막지 않습니다.

| Claude Code 훅 | 상태 |
|---|---|
| `UserPromptSubmit`, `PreToolUse`, `PostToolUse` | Running (`PreToolUse`는 도구 이름 표시) |
| `PreToolUse`(`AskUserQuestion`, `ExitPlanMode`), `PermissionRequest`, `Notification`, `Elicitation` | Needs input |
| `Stop`, `SessionStart`(자동 압축 `compact` 제외) | Idle |
| `StopFailure` | Error |
| `SessionEnd` | 상태 지움 |

cmux는 각 창의 셸에 `CMUX_WORKSPACE_ID`, `CMUX_SURFACE_ID`, `CMUX_SOCKET_PATH`를 넣어 줍니다. 다른 프로그램도 상태를 바꿀 수 있습니다:

```bash
cmux set-status running --message "빌드 중"   # running | needs_input | idle | error | clear
cmux set-status clear
cmux list-workspaces --json                   # status, status_message, unread 포함
```

워크스페이스의 알림 점(읽지 않음)은 그 워크스페이스로 이동하면 사라집니다. 사이드바 행을 오른쪽 클릭해서 **Mark as Unread / Mark as Read**로 직접 바꿀 수도 있습니다.

## 알림

Claude Code 등이 보내는 OSC 9/777 알림, 터미널 벨, `cmux notify` 명령이 같은 경로로 처리됩니다.

- 알림을 보낸 창과 워크스페이스에 사이드바 점이 표시됩니다.
- 그 창을 보고 있을 때(cmux 창이 활성이고 그 창에 포커스)는 알림을 띄우지 않습니다. 벨은 cmux 창이 비활성일 때만 알림을 띄웁니다.
- Windows 알림을 클릭하면 해당 워크스페이스로 이동하고 cmux 창이 앞으로 옵니다.
- 스크립트에서 보내기: `cmux notify --title "빌드" --body "끝났습니다" [--surface <UUID>] [--workspace <UUID>]`. 소켓 메서드는 `notification.create`.

`~/.config/cmux/config.toml`:

```toml
[notifications]
enabled = true                # 데스크톱 알림 표시. 꺼도 사이드바 점은 표시됨
suppress_when_focused = true  # 보고 있는 창에서 온 OSC/cmux notify 알림은 무시
bell = true                   # cmux 창이 비활성일 때 벨 알림 표시
# 알림마다 실행할 명령 (sh -c, 기다리지 않음). 환경변수:
# CMUX_NOTIFICATION_TITLE, CMUX_NOTIFICATION_BODY, CMUX_NOTIFICATION_KIND(bell|osc|cli),
# CMUX_WORKSPACE_ID, CMUX_WORKSPACE_NAME, CMUX_SURFACE_ID
command = 'echo "$CMUX_NOTIFICATION_TITLE" >> ~/cmux-notifications.log'
```

## 브라우저 미리보기

`Ctrl+Shift+B`로 현재 창 오른쪽에 브라우저 미리보기 창이 열립니다. 헤드리스 Chrome 화면을 받아 보여 주는 방식이라, 먼저 `contrib/wsl/setup.sh --skip-build --browser`로 Chrome을 설치해야 합니다.

- 주소창에 주소를 넣고 `Enter`(`https://`는 자동으로 붙음). 페이지를 클릭·스크롤·타이핑하면 Chrome으로 전달됩니다.
- 미리보기는 워크스페이스 전체를 채운 상태로 열립니다. 주소창 줄의 `⤡`/`⤢` 버튼이나 `Ctrl+Shift+Enter`로 터미널과 나란히 보기 / 전체 보기를 오갑니다.
- 닫기: 주소창 줄 오른쪽 끝의 `✕`, `Ctrl+Shift+Q`, 또는 오른쪽 클릭 → **Close Browser**.
- 디버깅 패널: 주소창 줄의 `{ }` 버튼이나 `F12`로 페이지 아래에 Console / Network / Elements / **Application** 탭이 열립니다. Application 탭에서는 Chrome DevTools처럼 Local·Session Storage, 쿠키(값 수정·삭제), IndexedDB, Cache Storage, Service Worker, Manifest를 보고 사이트 데이터를 지울 수 있습니다.
- 에이전트가 `cmux browser ...` 명령으로 같은 브라우저를 조작할 수 있습니다(`cmux browser --help`).

## 원본과 다른 점

`ime-support` 브랜치 = 원본 `main` + 빌드 수정 [PR #4](https://github.com/bradwilson331/cmux-linux/pull/4)(peteonrails) + 아래 변경. 원본 `main`은 ghostty 서브모듈이 존재하지 않는 커밋을 가리켜 빌드가 안 됩니다([#1](https://github.com/bradwilson331/cmux-linux/issues/1), [#3](https://github.com/bradwilson331/cmux-linux/issues/3)).

### WSL2에서 쓰려고 고친 것

| 문제 | 원인 | 커밋 |
|---|---|---|
| 한글 등 IME 입력 불가 | 키 입력이 입력기(`GtkIMContext`)를 거치지 않음. Ghostty GTK와 같은 방식으로 preedit/commit 연결 | `6b9f1962` |
| 복사/붙여넣기 안 됨 | 앱 단축키가 대상 터미널을 잘못 찾음(워크스페이스 간 `pane_id` 중복), 클립보드 콜백 ABI가 Ghostty와 불일치 | `21a33345` |
| `Ctrl+글자`가 앱에 안 감 (Claude Code `Ctrl+V` 이미지 등) | 키 이벤트의 `unshifted_codepoint`를 0으로 보내서 kitty 키보드 프로토콜 인코딩 실패 | `37510a52` |
| 알림이 안 옴 | OSC 9/777 알림(Claude Code가 사용)을 무시함 | `c36e923e` |
| 클릭으로 포커스 이동 불가 | 클릭 시 활성 창이 바뀌지 않아 포커스가 이전 창으로 되돌아감 | `af59e492` |
| 스크롤 방향 반대 | Ghostty GTK와 달리 스크롤 값 부호를 뒤집지 않음 | `093e78e0` |
| 창 닫기 시 크래시 | 터미널을 해제한 뒤 포커스 해제 이벤트가 해제된 터미널에 접근 | `9e8436de` |
| 가운데 창이 멈춘 채 남음 | 중첩 분할을 접을 때 위젯을 기존 부모에서 떼지 않아 GTK가 교체를 거부 | `2cf95066` |
| 가끔 키 입력이 통째로 안 됨 | 입력기가 빈 조합 문자열을 보내면 "조합 중" 상태가 풀리지 않아 모든 키가 버려짐 | `97909cb7` |
| 브라우저 창을 닫을 수 없음, 주소창 클릭 불가 | 닫기가 브라우저 프로그램만 끄고 창은 남김. 좁은 창에서 주소창이 버튼에 밀려 몇 픽셀로 줄어듦 | `5093f012` |

### macOS cmux v0.65 기능 가져오기

원본 Linux 포팅은 macOS cmux 0.62.2(2026년 3월) 무렵에서 갈라졌습니다. 그 뒤 macOS cmux v0.65.2까지 늘어난 기능 중 Linux에서 의미 있는 것을 가져왔습니다(포크 PR [#1](https://github.com/jhcheon-dwc/cmux-linux/pull/1)~[#6](https://github.com/jhcheon-dwc/cmux-linux/pull/6)).

| 기능 | 내용 | 자세히 |
|---|---|---|
| 창마다 가로 탭 | 새 탭, 전환, 닫기, 이름 바꾸기, 드래그로 순서 변경, 세션 복원 | [탭](#탭) |
| Claude Code 상태 표시 | 사이드바에 실행 중 / 입력 필요 / 대기 / 오류, 진행률, 읽음 표시 | [Claude Code 상태 표시](#claude-code-상태-표시) |
| 알림 | 사이드바 점, 보고 있는 창이면 생략, 사용자 훅, `cmux notify`, 알림 클릭 시 해당 워크스페이스로 이동 | [알림](#알림) |
| 창 이동 | 번호로 이동, 직전 창으로 돌아가기, 키보드로 분할선 이동, 균등 분할, 창 확대(zoom), 작업 디렉터리 상속 | [단축키](#단축키) |
| 세션 복원 | 재부팅 후에도 워크스페이스·분할·탭과 **각 창의 작업 디렉터리** 복원 (`cd`하면 5초 안에 저장) | — |
| 터미널 | 찾기 바(`Ctrl+Shift+F`), 링크 열기(Windows 브라우저), 창 제목 표시 | — |
| 소켓/CLI | `send-text`/`read-text`, `paste --submit`, 키 조합 전송(`send-key ctrl+c`), `current`, `read-selection`, 이전 워크스페이스 | `cmux --help` |

UI는 [DESIGN.md](DESIGN.md)(Google Labs의 [DESIGN.md 형식](https://github.com/google-labs-code/design.md))에 정의한 디자인 시스템 "cmux Signal"을 따릅니다. 크롬은 터미널 배경과 같은 계열의 슬레이트 단계로 통일하고, 색은 에이전트 상태 신호(응답 필요 앰버, 실행 중 청록, 오류 빨강)에만 씁니다. 토큰은 `src/style/tokens.css`, 컴포넌트 스타일은 `src/style/components.css`에 있습니다. 검사: `npx @google/design.md lint DESIGN.md`.

Cloud·계정·iOS 연동, Sparkle 업데이트, 메뉴 막대처럼 macOS나 Cloud 전용인 기능은 가져오지 않았습니다. 명령 팔레트, 전체 검색(세션·스크롤백), 워크스페이스 그룹, 재시작 시 Claude Code 세션 이어가기 등은 아직 없습니다.

원본 저장소에는 IME 지원을 [이슈 #5](https://github.com/bradwilson331/cmux-linux/issues/5)와 [PR #6](https://github.com/bradwilson331/cmux-linux/pull/6)으로 올려 두었습니다.

## 알려진 제한

- 기본 단축키 `Ctrl+N`, `Ctrl+B`, `Ctrl+D`, `Ctrl+[`(Esc), `Ctrl+]`, `Ctrl+1`~`9`, `Alt+1`~`9`(셸의 숫자 인자), `Ctrl+Tab`이 터미널 앱의 키와 겹칩니다. `[shortcuts]`에서 바꿀 수 있습니다.
- 탭은 같은 창 안에서만 옮길 수 있습니다(다른 창이나 워크스페이스로 끌어 옮기기는 없음).
- WSLg에서는 GTK 키·마우스 입력을 자동으로 만들 수 없어서, 새 기능은 소켓 CLI·D-Bus와 단위 테스트로 검증했습니다. 단축키와 마우스 동작은 실제로 눌러서 확인한 것만 보장합니다.
- 한 번도 화면에 표시되지 않은 창(백그라운드 워크스페이스에서 소켓으로 만든 창 등)은 소켓 `close-surface`로 닫히지 않을 수 있습니다.
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

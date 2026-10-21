#!/usr/bin/env bash
# cmux-linux (jhcheon-dwc fork) WSL2 설치 스크립트
#
# 이 PC에서 쓰는 것과 같은 상태로 맞춥니다:
#   1. 빌드/런타임 패키지 설치 (Rust, GTK4, Go, ibus-hangul, 한글/심볼 폰트 등)
#   2. cmux 빌드 → .deb 패키지 → 설치
#   3. 사용자 설정: 실행 래퍼, notify-send(Windows 토스트), 한글 입력 셸 설정, 다크 테마, Ghostty 설정
#   4. 시작 메뉴 항목이 래퍼를 쓰도록 수정
#   5. Windows 쪽: AutoHotkey로 한/영키 → Shift+Space, 마우스 휠 가속 (WSLg 창에서만)
#   6. Windows: 알림을 클릭하면 해당 워크스페이스로 이동 (cmux-focus: URI 등록)
#
# 여러 번 실행해도 됩니다. 이미 있는 Ghostty/GTK 설정 파일은 덮어쓰지 않습니다.
#
# 사용법: contrib/wsl/setup.sh [--skip-build] [--no-windows]
set -euo pipefail

SKIP_BUILD=0
WINDOWS=1
for arg in "$@"; do
    case "$arg" in
        --skip-build) SKIP_BUILD=1 ;;
        --no-windows) WINDOWS=0 ;;
        -h|--help) sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "알 수 없는 옵션: $arg" >&2; exit 1 ;;
    esac
done

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HERE="$REPO/contrib/wsl"
DESKTOP_FILE=/usr/share/applications/com.cmux_lx.terminal.desktop

step() { printf '\n\033[1;34m==> %s\033[0m\n' "$*"; }
warn() { printf '\033[1;33m경고: %s\033[0m\n' "$*" >&2; }

if ! grep -qi microsoft /proc/sys/kernel/osrelease 2>/dev/null; then
    warn "WSL이 아닌 것 같습니다. WSLg 전용 설정(d3d12, Windows 토스트, AutoHotkey)은 의미가 없을 수 있습니다."
fi

# ── 1. 패키지 ────────────────────────────────────────────────────────────────
step "패키지 설치"
sudo apt-get update
sudo apt-get install -y \
    git cargo rustc pkg-config libgtk-4-dev libclang-dev golang-go \
    ibus ibus-hangul ibus-gtk4 dbus-x11 x11-xkb-utils wl-clipboard language-pack-ko \
    fonts-noto-cjk fonts-nanum fonts-noto-core fonts-noto-color-emoji

# ── 2. 빌드 + 설치 ───────────────────────────────────────────────────────────
if [ "$SKIP_BUILD" -eq 0 ]; then
    step "cmux 빌드 (처음에는 Zig 다운로드와 libghostty 빌드로 오래 걸립니다)"
    cd "$REPO"
    git submodule update --init --recursive
    ./scripts/setup-linux.sh
    cargo build --release --workspace
    (cd daemon/remote && go build -o cmuxd-remote ./cmd/cmuxd-remote)

    step ".deb 패키지 생성 및 설치"
    ./packaging/scripts/build-deb.sh
    VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
    # 같은 버전 재설치도 반영되도록 apt install 대신 dpkg -i 사용
    sudo dpkg -i "dist/cmux_${VERSION}_amd64.deb" || sudo apt-get install -f -y
fi

# ── 3. 사용자 설정 ───────────────────────────────────────────────────────────
step "실행 래퍼, notify-send, 한글 입력 설정 설치"
install -Dm755 "$HERE/bin/cmux-app" "$HOME/.local/bin/cmux-app"
install -Dm755 "$HERE/bin/notify-send" "$HOME/.local/bin/notify-send"
install -Dm644 "$HERE/shell/ime.sh" "$HOME/.config/cmux-wsl/ime.sh"

SOURCE_LINE='[ -f "$HOME/.config/cmux-wsl/ime.sh" ] && . "$HOME/.config/cmux-wsl/ime.sh"  # cmux-linux WSL'
for rc in "$HOME/.zshrc" "$HOME/.bashrc"; do
    [ -f "$rc" ] || continue
    if ! grep -qF '# cmux-linux WSL' "$rc"; then
        printf '\n%s\n' "$SOURCE_LINE" >> "$rc"
        echo "  $rc 에 등록"
    fi
done

install_if_missing() {  # src dst: dst가 없거나 비어 있을 때만 설치
    if [ ! -s "$2" ]; then
        install -Dm644 "$1" "$2"
        echo "  $2 설치"
    else
        echo "  $2 는 이미 있어서 그대로 둠"
    fi
}
install_if_missing "$HERE/config/gtk/settings.ini" "$HOME/.config/gtk-4.0/settings.ini"
install_if_missing "$HERE/config/gtk/settings.ini" "$HOME/.config/gtk-3.0/settings.ini"
install_if_missing "$HERE/config/ghostty/config.ghostty" "$HOME/.config/ghostty/config.ghostty"

# ── 4. 시작 메뉴 ─────────────────────────────────────────────────────────────
if [ -f "$DESKTOP_FILE" ]; then
    step "시작 메뉴 항목이 래퍼를 쓰도록 수정"
    mkdir -p "$HOME/.local/share/applications"
    sed "s|^Exec=.*|Exec=$HOME/.local/bin/cmux-app|" "$DESKTOP_FILE" \
        > "$HOME/.local/share/applications/com.cmux_lx.terminal.desktop"
    # WSLg는 시스템 항목을 쓰므로 그것도 수정 (패키지를 다시 설치하면 원래대로 돌아가니 이 스크립트를 다시 실행)
    sudo sed -i "s|^Exec=.*|Exec=$HOME/.local/bin/cmux-app|" "$DESKTOP_FILE"
fi

# ── 5. Windows: 한/영키 → Shift+Space, 마우스 휠 가속 ────────────────────────
if [ "$WINDOWS" -eq 1 ] && command -v powershell.exe >/dev/null 2>&1; then
    step "Windows: AutoHotkey 입력 보정 스크립트(wslg-input.ahk) 설치"
    WIN_HOME_W=$(powershell.exe -NoProfile -Command '$env:USERPROFILE' | tr -d '\r')
    WIN_LOCAL_W=$(powershell.exe -NoProfile -Command '$env:LOCALAPPDATA' | tr -d '\r')
    WIN_HOME=$(wslpath "$WIN_HOME_W")
    AHK_EXE_W="$WIN_LOCAL_W\\Programs\\AutoHotkey\\v2\\AutoHotkey64.exe"
    if [ ! -f "$(wslpath "$AHK_EXE_W")" ]; then
        echo "  AutoHotkey v2 설치 (winget)"
        winget.exe install --id AutoHotkey.AutoHotkey -e --silent \
            --accept-package-agreements --accept-source-agreements || warn "AutoHotkey 설치 실패. 직접 설치 후 다시 실행하세요."
    fi
    cp "$HERE/windows/wslg-input.ahk" "$WIN_HOME/wslg-input.ahk"
    rm -f "$WIN_HOME/hangul-wslg.ahk"  # 예전 버전 파일
    AHK_SCRIPT_W="$WIN_HOME_W\\wslg-input.ahk"
    # 예전 버전의 한/영키 전용 스크립트(hangul-wslg.ahk)는 정리하고, 새 스크립트를
    # 시작프로그램에 등록한 뒤 바로 실행
    powershell.exe -NoProfile -Command "
        Get-CimInstance Win32_Process -Filter \"Name='AutoHotkey64.exe'\" |
            Where-Object { \$_.CommandLine -match 'hangul-wslg|wslg-input' } |
            ForEach-Object { Stop-Process -Id \$_.ProcessId -Force }
        \$startup = Join-Path \$env:APPDATA 'Microsoft\\Windows\\Start Menu\\Programs\\Startup'
        Remove-Item (Join-Path \$startup 'hangul-wslg.lnk') -ErrorAction SilentlyContinue
        \$s = (New-Object -ComObject WScript.Shell).CreateShortcut((Join-Path \$startup 'wslg-input.lnk'))
        \$s.TargetPath = '$AHK_EXE_W'
        \$s.Arguments = '\"$AHK_SCRIPT_W\"'
        \$s.Save()
        if (Test-Path '$AHK_EXE_W') { Start-Process '$AHK_EXE_W' -ArgumentList '\"$AHK_SCRIPT_W\"' }
    " >/dev/null
    echo "  시작프로그램 등록: wslg-input.lnk"
fi

# ── 6. Windows: 알림 클릭 → 워크스페이스 이동 (notifications) ────────────────
# notify-send 셰임이 만드는 토스트는 클릭하면 cmux-focus:<워크스페이스 UUID>를 엽니다.
# 그 URI를 처리할 cmux-focus.ps1을 HKCU\Software\Classes\cmux-focus에 등록합니다.
if [ "$WINDOWS" -eq 1 ] && command -v powershell.exe >/dev/null 2>&1; then
    step "Windows: 알림 클릭 처리기(cmux-focus:) 등록"
    # 5단계에서 이미 구했으면 재사용
    WIN_LOCAL_W=${WIN_LOCAL_W:-$(powershell.exe -NoProfile -Command '$env:LOCALAPPDATA' | tr -d '\r')}
    FOCUS_DIR="$(wslpath "$WIN_LOCAL_W")/cmux"
    mkdir -p "$FOCUS_DIR"
    cp "$HERE/windows/cmux-focus.ps1" "$FOCUS_DIR/cmux-focus.ps1"
    CMUX_CLI=$(command -v cmux 2>/dev/null || echo /usr/bin/cmux)
    DISTRO_ARGS=()  # 빈 인자는 WSL에서 Windows로 넘어가며 사라질 수 있어 따로 둠
    [ -n "${WSL_DISTRO_NAME:-}" ] && DISTRO_ARGS=(-Distro "$WSL_DISTRO_NAME")
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$WIN_LOCAL_W\\cmux\\cmux-focus.ps1" \
        -Register "${DISTRO_ARGS[@]}" -Cmux "$CMUX_CLI" \
        || warn "cmux-focus 등록 실패. 알림 클릭으로 워크스페이스 이동이 안 됩니다."
    echo "  HKCU\\Software\\Classes\\cmux-focus → $WIN_LOCAL_W\\cmux\\cmux-focus.ps1"
fi

# ── 마무리 ──────────────────────────────────────────────────────────────────
step "완료"
cat <<'EOF'
다음을 해 주세요:
  1. 새 터미널을 열거나 `source ~/.zshrc` (ibus 데몬과 한글 엔진이 준비됩니다)
  2. cmux 실행: 터미널에서 `cmux-app`, 또는 Windows 시작 메뉴의 cmux
     (/usr/bin/cmux-app을 직접 실행하면 한글 입력이 안 됩니다)
  3. 한/영 전환: Shift+Space 또는 한/영키
EOF

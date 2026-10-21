# cmux-linux WSL 설정: 한글 입력(ibus-hangul, WSLg)
# contrib/wsl/setup.sh가 ~/.zshrc, ~/.bashrc에서 이 파일을 source하도록 등록합니다.

export PATH="$HOME/.local/bin:$PATH"

export GTK_IM_MODULE=ibus
export QT_IM_MODULE=ibus
export XMODIFIERS=@im=ibus
export LANG=ko_KR.UTF-8

# 연결이 안 되면 데몬을 띄움. -r(기존 데몬 대체)은 쓰지 않음: 이미 떠 있으면 새 데몬이 바로 종료되어 중복이 안 생김
if command -v ibus-daemon >/dev/null && [ -n "$WAYLAND_DISPLAY$DISPLAY" ] && ! ibus engine >/dev/null 2>&1; then
    ibus-daemon -dx >/dev/null 2>&1
    # ibus-memconf는 설정을 저장하지 않아 새 데몬은 영문 엔진으로 시작함 → 한글 엔진 지정
    # (Shift+Space 한/영 전환은 한글 엔진 안에서만 동작)
    for _ in 1 2 3 4 5; do ibus engine hangul >/dev/null 2>&1 && break; sleep 0.5; done
fi

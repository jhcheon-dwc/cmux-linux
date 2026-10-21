#Requires AutoHotkey v2.0
#SingleInstance Force
; 터치패드 스크롤은 짧은 간격으로 휠 이벤트를 많이 보내서 기본 한도(2초에 70번)를 금방 넘고
; "71 hotkeys have been received..." 경고 창이 뜸. 휠/터치패드 판별만 하고 통과시키므로 한도를 높임.
A_MaxHotkeysPerInterval := 2000

; WSLg 창(msrdc.exe)용 입력 보정. contrib/wsl/setup.sh가 시작프로그램에 등록합니다.

; ── 한/영키 → Shift+Space ───────────────────────────────────────────────────
; WSLg는 한/영키를 Linux로 전달하지 않으므로 ibus-hangul 전환 키로 바꿈
#HotIf WinActive("ahk_exe msrdc.exe")
vk15::Send "+{Space}"
#HotIf

; ── 마우스 휠만 빠르게 ──────────────────────────────────────────────────────
; WSLg에서는 Ghostty 설정(mouse-scroll-multiplier)으로 휠과 터치패드 속도를 따로 맞추기
; 어려움(휠 스크롤의 첫 이벤트만 정밀 스크롤로 처리되는 등). Windows에서는 둘이 구분됨:
; 휠은 120(한 칸)의 배수, 터치패드는 120의 배수가 아닌 작은 값으로 들어옴.
; 원래 이벤트는 그대로 통과(~)시키고 휠일 때만 칸을 더 보냄. 터치패드는 건드리지 않음.
WheelFactor := 2  ; 휠 한 칸을 몇 칸으로 만들지 (1이면 그대로)

MouseOverWslg() {
    try {
        MouseGetPos ,, &win
        return WinGetProcessName("ahk_id " win) = "msrdc.exe"
    }
    return false
}

#HotIf MouseOverWslg()
~WheelUp::BoostWheel("WheelUp")
~WheelDown::BoostWheel("WheelDown")
#HotIf

BoostWheel(button) {
    delta := A_EventInfo  ; 이번 이벤트의 스크롤 양 (휠 한 칸 = 120)
    if (WheelFactor <= 1 || delta = 0 || Mod(delta, 120) != 0)
        return  ; 터치패드(정밀 스크롤)는 그대로 둠
    extra := (WheelFactor - 1) * (delta // 120)
    Send "{" button " " extra "}"
}

#Requires AutoHotkey v2.0
#SingleInstance Force

; WSLg(msrdc.exe) 창에서만 한/영키를 Shift+Space로 변환 (ibus-hangul 전환용)
#HotIf WinActive("ahk_exe msrdc.exe")
vk15::Send "+{Space}"
#HotIf

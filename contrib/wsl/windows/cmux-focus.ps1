# cmux-focus: URI handler for clicked cmux toast notifications (Windows side).
#
# The WSL notify-send shim builds toasts with launch="cmux-focus:<workspace-uuid>".
# setup.sh registers the cmux-focus URI scheme (HKCU\Software\Classes\cmux-focus) to run:
#   powershell -WindowStyle Hidden -ExecutionPolicy Bypass -File cmux-focus.ps1 -Distro <distro> -Cmux <path> "%1"
# This selects the workspace through the cmux CLI inside WSL and brings the cmux
# window (a WSLg window owned by msrdc.exe) to the front.
#
# Register/unregister the scheme (run by setup.sh; -Scheme lets tests use another name):
#   cmux-focus.ps1 -Register -Distro <distro> -Cmux <path> [-Scheme cmux-focus]
#   cmux-focus.ps1 -Unregister [-Scheme cmux-focus]
param(
    [Parameter(Position = 0)][string]$Uri = '',
    [string]$Distro = '',
    [string]$Cmux = '/usr/bin/cmux',
    # Optional socket path (default: the CLI's own discovery)
    [string]$Socket = '',
    # Window title to raise (a WSLg window owned by msrdc.exe)
    [string]$Title = 'cmux',
    [switch]$Register,
    [switch]$Unregister,
    [string]$Scheme = 'cmux-focus'
)

if ($Register -or $Unregister) {
    if ($Scheme -notmatch '^[a-z][a-z0-9-]*$') { exit 2 }
    $key = "HKCU:\Software\Classes\$Scheme"
    if ($Unregister) {
        Remove-Item -Path $key -Recurse -Force -ErrorAction SilentlyContinue
        exit 0
    }
    $ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $cmd = "`"$ps`" -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$PSCommandPath`""
    if ($Distro) { $cmd += " -Distro `"$Distro`"" }
    $cmd += " -Cmux `"$Cmux`""
    if ($Socket) { $cmd += " -Socket `"$Socket`"" }
    $cmd += ' "%1"'
    New-Item -Path "$key\shell\open\command" -Force | Out-Null
    Set-Item -Path $key -Value 'URL:cmux focus'
    New-ItemProperty -Path $key -Name 'URL Protocol' -Value '' -Force | Out-Null
    Set-Item -Path "$key\shell\open\command" -Value $cmd
    exit 0
}

if ($Uri -notmatch '^[A-Za-z][A-Za-z0-9+.-]*:/*([0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12})/?$') {
    exit 2
}
$workspace = $Matches[1]

$wslArgs = @()
if ($Distro) { $wslArgs += @('-d', $Distro) }
$wslArgs += @('--', $Cmux)
if ($Socket) { $wslArgs += @('--socket', $Socket) }
$wslArgs += @('select-workspace', $workspace)
& wsl.exe @wslArgs
$status = $LASTEXITCODE

Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class CmuxFocusWin {
    delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr lParam);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr hWnd, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
    [DllImport("user32.dll")] static extern bool IsIconic(IntPtr hWnd);
    [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr hWnd, int cmd);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    // Title is `title` itself or starts with `title ` (WSLg appends " (<distro>)").
    public static bool TitleMatches(string text, string title) {
        return text == title || text.StartsWith(title + " ", StringComparison.Ordinal);
    }
    // Visible top-level window of one of `pids` whose title matches `title`.
    public static IntPtr Find(uint[] pids, string title) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            if (!IsWindowVisible(h)) return true;
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (Array.IndexOf(pids, pid) < 0) return true;
            var sb = new StringBuilder(512); GetWindowText(h, sb, 512);
            if (!TitleMatches(sb.ToString(), title)) return true;
            found = h; return false;
        }, IntPtr.Zero);
        return found;
    }
    public static bool Raise(IntPtr h) {
        if (IsIconic(h)) ShowWindow(h, 9); // SW_RESTORE
        if (SetForegroundWindow(h)) return true;
        // Foreground lock: a synthetic Alt press lets this process take the foreground.
        keybd_event(0x12, 0, 0, UIntPtr.Zero);     // VK_MENU down
        keybd_event(0x12, 0, 2, UIntPtr.Zero);     // VK_MENU up (KEYEVENTF_KEYUP)
        return SetForegroundWindow(h);
    }
}
'@

$pids = [uint32[]]@(Get-Process -Name msrdc -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
if ($pids.Count -gt 0) {
    $hwnd = [CmuxFocusWin]::Find($pids, $Title)
    if ($hwnd -ne [IntPtr]::Zero) {
        $null = [CmuxFocusWin]::Raise($hwnd)
    }
}
exit $status

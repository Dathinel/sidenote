$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath("$PSScriptRoot/..")
$output=Join-Path $root 'artifacts/floating-icon-spike'
$source=Get-Content -Raw -LiteralPath "$PSScriptRoot/floating-icon-proof.ps1"
$csharp=[regex]::Match($source,"(?s)Add-Type -TypeDefinition @'\r?\n(.*?)\r?\n'@").Groups[1].Value
Add-Type -TypeDefinition $csharp
Add-Type -TypeDefinition @'
using System; using System.Text; using System.Runtime.InteropServices;
public static class SpikeMenu {
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetMenuStringW(IntPtr menu,uint item,StringBuilder text,int size,uint flags);
    public static string Label(IntPtr menu) {var text=new StringBuilder(256); GetMenuStringW(menu,0,text,256,0x400); return text.ToString();}
}
'@
function Wait-Panel($process) {
    for($i=0;$i -lt 40;$i++) {
        foreach($window in [IconProof]::Windows($process.Id)) {
            if([IconProof]::Title($window) -eq 'Sidenote floating panel spike'){return $window}
        }
        Start-Sleep -Milliseconds 50
    }
    throw 'Panel did not open'
}
function Pick-Tray($process,$tray,$downCount) {
    [void][IconProof]::PostMessageW($tray,0x8001,[UIntPtr]::new([uint64]1),[IntPtr]0x0205)
    Start-Sleep -Milliseconds 300
    $popup=[IntPtr]::Zero
    foreach($window in [IconProof]::Windows($process.Id)) {
        $menu=[IconProof]::SendMessageW($window,0x01E1,[UIntPtr]::Zero,[IntPtr]::Zero)
        if($menu -ne [IntPtr]::Zero) {
            if([SpikeMenu]::Label($menu) -ne 'Show meeting panel'){throw 'Wrong native popup label'}
            $popup=$window
            break
        }
    }
    if($popup -eq [IntPtr]::Zero){throw 'Rune native tray popup not found'}
    # Target only this process's popup queue; no global keyboard input or Explorer actions.
    for($i=0;$i -lt $downCount;$i++) {
        [void][IconProof]::PostMessageW($popup,0x0100,[UIntPtr]::new([uint64]40),[IntPtr]0x01500001)
        Start-Sleep -Milliseconds 80
    }
    [void][IconProof]::PostMessageW($popup,0x0100,[UIntPtr]::new([uint64]13),[IntPtr]0x001C0001)
}
$process=$null
try {
    $process=Start-Process -FilePath "$root/target/x86_64-pc-windows-gnu/debug/meeting-icon-spike.exe" -ArgumentList '--hybrid','--seconds','30' -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $output 'tray-menu.stdout.log') -RedirectStandardError (Join-Path $output 'tray-menu.stderr.log')
    Start-Sleep -Seconds 2
    $tray=[IconProof]::Tray($process.Id)
    if($tray -eq [IntPtr]::Zero -or [IconProof]::TrayRect($tray) -ne 0){throw 'Tray not registered'}
    Pick-Tray $process $tray 1
    $panel=Wait-Panel $process
    $affinity=[IconProof]::Affinity($panel)
    if($affinity -ne 17){throw 'Tray-opened panel lost capture exclusion'}
    $log=Get-Content -Raw -LiteralPath (Join-Path $output 'tray-menu.stdout.log')
    if(!$log.Contains('tray event Menu("show-panel")')){throw 'Show menu handler was not invoked'}
    Pick-Tray $process $tray 2
    if(!$process.WaitForExit(3000)){throw 'Tray Quit did not exit'}
    $removed=[IconProof]::TrayRect($tray) -ne 0
    if(!$removed){throw 'Tray icon did not unregister'}
    $result=[ordered]@{nativeMenuLabel='Show meeting panel';menuHandlerInvoked=$true;panelOpened=$true;panelAffinity=$affinity;quitMenuExited=$true;trayRemoved=$removed}
    $result | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $output 'tray-proof.json')
    $result | ConvertTo-Json
} finally {
    if($process -and !$process.HasExited){Stop-Process -Id $process.Id; $process.WaitForExit()}
}

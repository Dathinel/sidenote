# Targeted native-event proof for a desktop where a foreign window covers the pointer.
$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath("$PSScriptRoot/..")
$output=Join-Path $root 'artifacts/floating-icon-spike'
$source=Get-Content -Raw -LiteralPath "$PSScriptRoot/floating-icon-proof.ps1"
$csharp=[regex]::Match($source,"(?s)Add-Type -TypeDefinition @'\r?\n(.*?)\r?\n'@").Groups[1].Value
Add-Type -TypeDefinition $csharp
[void][IconProof]::SetProcessDpiAwarenessContext([IntPtr](-4))
function Find-Own($process,$title) {
    for($i=0;$i -lt 60;$i++) {
        foreach($window in [IconProof]::Windows($process.Id)) {
            if([IconProof]::Title($window) -eq $title){return $window}
        }
        Start-Sleep -Milliseconds 100
    }
    throw "No own window: $title"
}
function Pointer-Message([IntPtr]$window,[uint32]$message,[int]$x,[int]$y) {
    $point=[IntPtr]::new([int64](($y -band 65535)*65536+($x -band 65535)))
    $button=if($message -eq 0x0201){[UIntPtr]::new([uint64]1)}else{[UIntPtr]::Zero}
    [void][IconProof]::SendMessageW($window,$message,$button,$point)
    Start-Sleep -Milliseconds 80
}
function Click-Own([IntPtr]$window,[int]$x,[int]$y) {
    Pointer-Message $window 0x0200 $x $y
    Pointer-Message $window 0x0201 $x $y
    Pointer-Message $window 0x0202 $x $y
    Start-Sleep -Milliseconds 400
}
$results=@()
$state=Join-Path $output 'message-position.txt'
[IO.File]::WriteAllText($state,'0.4')
foreach($shape in @('bead','pill','hybrid','restart')) {
    $process=$null
    try {
        $arguments=@('--state',('"'+$state+'"'),'--seconds','25')
        if($shape -eq 'pill'){$arguments+='--pill'}
        if($shape -eq 'hybrid'){$arguments+='--hybrid'}
        $log=Join-Path $output "message-$shape.stdout.log"
        $process=Start-Process -FilePath "$root/target/x86_64-pc-windows-gnu/debug/meeting-icon-spike.exe" -ArgumentList $arguments -PassThru -WindowStyle Hidden -RedirectStandardOutput $log -RedirectStandardError (Join-Path $output "message-$shape.stderr.log")
        $icon=Find-Own $process 'Sidenote floating icon spike'
        Start-Sleep -Seconds 10
        $before=[IconProof]::Bounds($icon)
        $memory=(Get-Process -Id $process.Id).WorkingSet64
        $initialFraction=[double]::Parse([IO.File]::ReadAllText($state),[Globalization.CultureInfo]::InvariantCulture)
        $scale=[IconProof]::GetDpiForWindow($icon)/96.0
        $height=$before.Bottom-$before.Top
        $width=$before.Right-$before.Left
        $primary=@([IconProof]::Monitors() | Where-Object {$_.Flags -band 1})[0]
        $expectedY=$primary.Work.Top+[int][Math]::Round((8+$initialFraction*(($primary.Work.Bottom-$primary.Work.Top)/$scale-$height/$scale-16))*$scale)
        if([Math]::Abs($before.Top-$expectedY) -gt 1){throw 'Restart position mismatch'}
        Click-Own $icon ([int]($width/2)) ([int]($height/2))
        $panel=Find-Own $process 'Sidenote floating panel spike'
        $panelBefore=[IconProof]::Bounds($panel)
        Pointer-Message $icon 0x0201 ([int]($width/2)) ([int]($height/2))
        $screenY=$before.Top+[int]($height/2)
        $hidden=$false
        for($step=1;$step -le 12;$step++) {
            $now=[IconProof]::Bounds($icon)
            Pointer-Message $icon 0x0200 ([int]($width/2)) ($screenY+8*$step-$now.Top)
            if($step -ge 3) {
                if([IconProof]::IsWindowVisible($panel)){throw 'Card stayed visible during drag'}
                $hidden=$true
            }
        }
        $now=[IconProof]::Bounds($icon)
        Pointer-Message $icon 0x0202 ([int]($width/2)) ($screenY+96-$now.Top)
        Start-Sleep -Milliseconds 500
        $after=[IconProof]::Bounds($icon)
        $panelAfter=[IconProof]::Bounds($panel)
        if($after.Top-$before.Top -ne 96){throw 'Native drag did not move 96 px'}
        if(![IconProof]::IsWindowVisible($panel)){throw 'Card did not reopen on drop'}
        $clampedY=[Math]::Min($after.Top,$primary.Work.Bottom-($panelAfter.Bottom-$panelAfter.Top))
        if($panelAfter.Top -ne $clampedY){throw 'Card drop placement was wrong'}
        $affinity=[IconProof]::Affinity($icon)
        $panelAffinity=[IconProof]::Affinity($panel)
        if($affinity -ne 17 -or $panelAffinity -ne 17){throw 'Capture affinity lost'}
        [void][IconProof]::PostMessageW($panel,0x0010,[UIntPtr]::Zero,[IntPtr]::Zero)
        Start-Sleep -Milliseconds 300
        if($process.HasExited -or ![IconProof]::IsWindowVisible($icon)){throw 'Close killed icon'}
        $tray=[IconProof]::Tray($process.Id)
        if([IconProof]::TrayRect($tray) -ne 0){throw 'Tray was not registered'}
        $results += [ordered]@{shape=$shape;workingSetBytes=$memory;dpi=96*$scale;width=$width;height=$height;initialY=$before.Top;dropY=$after.Top;panelDropY=$panelAfter.Top;panelHiddenDuringDrag=$hidden;panelReopened=$true;iconAffinity=$affinity;panelAffinity=$panelAffinity;savedFraction=[IO.File]::ReadAllText($state);restartPositionVerified=$true;eventMethod='WM_MOUSEMOVE/WM_LBUTTONDOWN/WM_LBUTTONUP targeted only to owned HWND'}
        if(!$process.WaitForExit(20000)){throw 'Timed own process did not exit'}
        if([IconProof]::TrayRect($tray) -eq 0){throw 'Tray registration survived exit'}
        $results | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $output 'window-message-proof.json')
    } finally {
        if($process -and !$process.HasExited){Stop-Process -Id $process.Id; $process.WaitForExit()}
    }
}
$results | ConvertTo-Json -Depth 5

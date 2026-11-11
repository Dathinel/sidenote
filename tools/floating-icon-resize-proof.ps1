param([switch]$Visual, [switch]$AtomicAnchor)
$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath("$PSScriptRoot/..")
$output=Join-Path $root 'artifacts/floating-icon-spike'
New-Item -ItemType Directory -Force -Path $output | Out-Null
$source=Get-Content -Raw -LiteralPath "$PSScriptRoot/floating-icon-proof.ps1"
$csharp=[regex]::Match($source,"(?s)Add-Type -TypeDefinition @'\r?\n(.*?)\r?\n'@").Groups[1].Value
Add-Type -TypeDefinition $csharp
[void][IconProof]::SetProcessDpiAwarenessContext([IntPtr](-4))
$mode=if($Visual){'visual'}else{'resize'}
if($AtomicAnchor){$mode+='-anchored'}
$log=Join-Path $output "$mode.stdout.log"
$arguments=@('--resize-proof','--seconds','34','--state',('"'+(Join-Path $output 'resize-position.txt')+'"'))
if($Visual){$arguments+='--visual-proof'}
if($AtomicAnchor){$arguments+='--atomic-anchor'}
$process=$null
$recorder=$null
try {
    $process=Start-Process -FilePath "$root/target/x86_64-pc-windows-gnu/debug/meeting-icon-spike.exe" -ArgumentList $arguments -PassThru -WindowStyle Hidden -RedirectStandardOutput $log -RedirectStandardError (Join-Path $output "$mode.stderr.log")
    $icon=[IntPtr]::Zero
    for($attempt=0; $attempt -lt 100 -and $icon -eq [IntPtr]::Zero; $attempt++) {
        foreach($window in [IconProof]::Windows($process.Id)) {
            if([IconProof]::Title($window) -eq 'Sidenote floating icon spike'){$icon=$window}
        }
        Start-Sleep -Milliseconds 100
    }
    if($icon -eq [IntPtr]::Zero){throw 'No spike icon'}
    $initial=[IconProof]::Bounds($icon)
    $scale=[IconProof]::GetDpiForWindow($icon)/96.0
    if($Visual) {
        # Record only the maximum placeholder bounds, never the desktop as a whole.
        $width=[int](104*$scale)
        $height=$initial.Bottom-$initial.Top
        $x=$initial.Right-$width
        $video=Join-Path $output 'hybrid-resize.mkv'
        $capture=@('-hide_banner','-loglevel','error','-f','gdigrab','-framerate','60','-draw_mouse','0','-offset_x',"$x",'-offset_y',"$($initial.Top)",'-video_size',"${width}x${height}",'-i','desktop','-t','29','-c:v','ffv1','-y',('"'+$video+'"'))
        $recorder=Start-Process -FilePath (Get-Command ffmpeg).Source -ArgumentList $capture -PassThru -WindowStyle Hidden -RedirectStandardError (Join-Path $output 'visual-capture.stderr.log')
    }
    $clock=[Diagnostics.Stopwatch]::StartNew()
    $cpuStart=(Get-Process -Id $process.Id).TotalProcessorTime.TotalMilliseconds
    $phase='idle'
    $samples=[Collections.Generic.List[object]]::new()
    while(!$process.HasExited -and $clock.Elapsed.TotalSeconds -lt 32) {
        $r=[IconProof]::Bounds($icon)
        $samples.Add([pscustomobject]@{ms=[Math]::Round($clock.Elapsed.TotalMilliseconds,2);x=$r.Left;y=$r.Top;width=$r.Right-$r.Left;height=$r.Bottom-$r.Top;right=$r.Right;visible=[IconProof]::IsWindowVisible($icon)})
        $text=Get-Content -Raw -LiteralPath $log
        if($phase -eq 'idle' -and $text.Contains('resize proof begin')) {
            $idleMs=$clock.Elapsed.TotalMilliseconds
            $cpuResizeStart=(Get-Process -Id $process.Id).TotalProcessorTime.TotalMilliseconds
            $idleCpuMs=$cpuResizeStart-$cpuStart
            $phase='resize'
        }
        if($phase -eq 'resize' -and $text.Contains('resize proof end')) {
            $resizeMs=$clock.Elapsed.TotalMilliseconds-$idleMs
            $resizeCpuMs=(Get-Process -Id $process.Id).TotalProcessorTime.TotalMilliseconds-$cpuResizeStart
            $phase='complete'
            break
        }
        Start-Sleep -Milliseconds 15
    }
    if($phase -ne 'complete'){throw 'Resize probe did not finish'}
    $rightValues=@($samples | ForEach-Object {$_.right} | Sort-Object -Unique)
    $widthValues=@($samples | ForEach-Object {$_.width} | Sort-Object -Unique)
    $summary=[ordered]@{mode=$mode;dpi=[IconProof]::GetDpiForWindow($icon);sampleCount=$samples.Count;
        physicalWidths=$widthValues;rightEdges=$rightValues;hiddenSamples=@($samples | Where-Object {!$_.visible}).Count;
        idleElapsedMs=$idleMs;idleCpuMs=$idleCpuMs;idlePercentOfOneCore=100*$idleCpuMs/$idleMs;
        resizeElapsedMs=$resizeMs;resizeCpuMs=$resizeCpuMs;resizePercentOfOneCore=100*$resizeCpuMs/$resizeMs;
        logicalProcessors=[Environment]::ProcessorCount;affinity=[IconProof]::Affinity($icon)}
    $samples | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $output "$mode-samples.json")
    $summary | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $output "$mode-proof.json")
    $summary | ConvertTo-Json -Depth 4
    if($rightValues.Count -ne 1){throw 'Right edge moved while resizing'}
    if($widthValues[0] -ne [int](44*$scale) -or $widthValues[-1] -ne [int](104*$scale)){throw 'Wrong resize endpoints'}
    if($summary.hiddenSamples -ne 0){throw 'Overlay hid during resize'}
    if(!$process.WaitForExit(10000)){throw 'Timed probe did not exit'}
    if($recorder -and !$recorder.WaitForExit(10000)){throw 'Recording did not finish'}
} finally {
    if($recorder -and !$recorder.HasExited){Stop-Process -Id $recorder.Id}
    if($process -and !$process.HasExited){Stop-Process -Id $process.Id; $process.WaitForExit()}
}

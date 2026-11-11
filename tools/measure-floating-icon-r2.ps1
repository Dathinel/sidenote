param(
    [Parameter(Mandatory=$true)][string]$Exe,
    [string]$Out = "$PSScriptRoot/../artifacts/floating-icon-r2"
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot/..").Path
$Exe = (Resolve-Path -LiteralPath $Exe).Path
$Out = [IO.Path]::GetFullPath($Out)
if (!$Out.StartsWith($root + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Output must stay in this worktree.' }
$run = Join-Path $Out ('run-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
$state = Join-Path $run 'data'
$local = Join-Path $state 'local'
$meetings = Join-Path $run 'meetings'
New-Item -ItemType Directory -Force -Path $local,$meetings,(Join-Path $run 'appdata'),(Join-Path $run 'localappdata') | Out-Null
[IO.File]::WriteAllText((Join-Path $state 'config.json'),'{"notes":{"engine":"off"}}')
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class FloatingR2 {
  public delegate bool Each(IntPtr hwnd, IntPtr data);
  [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
  [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; public Point(int x,int y) { X=x;Y=y; } }
  [DllImport("user32.dll")] public static extern bool EnumWindows(Each callback, IntPtr data);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr hwnd, StringBuilder text, int count);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool GetWindowDisplayAffinity(IntPtr hwnd, out uint value);
  [DllImport("user32.dll")] public static extern IntPtr GetWindowLongPtrW(IntPtr hwnd,int index);
  [DllImport("user32.dll")] public static extern int GetWindowRgn(IntPtr hwnd,IntPtr region);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point point);
  [DllImport("gdi32.dll")] public static extern IntPtr CreateRectRgn(int a,int b,int c,int d);
  [DllImport("gdi32.dll")] public static extern bool PtInRegion(IntPtr region,int x,int y);
  [DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr obj);
  [DllImport("user32.dll")] public static extern uint GetGuiResources(IntPtr process,uint kind);
  [DllImport("user32.dll")] public static extern IntPtr SendMessageTimeoutW(IntPtr hwnd,uint msg,IntPtr w,IntPtr l,uint flags,uint timeout,out IntPtr result);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd,IntPtr dc,uint flags);
  [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
  public static IntPtr[] Windows(uint owned) {
    var found = new System.Collections.Generic.List<IntPtr>();
    EnumWindows((h,d) => { uint p;GetWindowThreadProcessId(h,out p);if(p==owned)found.Add(h);return true; },IntPtr.Zero);
    return found.ToArray();
  }
  public static string Class(IntPtr h) { var s=new StringBuilder(256);GetClassName(h,s,s.Capacity);return s.ToString(); }
  public static void Message(IntPtr h,uint owned,uint msg) {
    uint p;GetWindowThreadProcessId(h,out p);if(p!=owned)throw new Exception("Refusing message to another process");
    IntPtr result;if(SendMessageTimeoutW(h,msg,IntPtr.Zero,IntPtr.Zero,2,2000,out result)==IntPtr.Zero)throw new Exception("Own-window message timed out");
  }
}
'@
[void][FloatingR2]::SetThreadDpiAwarenessContext([IntPtr](-4))
$utf8 = New-Object Text.UTF8Encoding($false)
function Write-Snapshot([bool]$Recording,[bool]$Speaking,[int]$Elapsed=754,[string]$Id='1794440000000-1') {
    $body = @{id=$Id;recording=$Recording;speaking=$Speaking;elapsed=$Elapsed} | ConvertTo-Json -Compress
    $path = Join-Path $local 'floating-icon-replay.json'
    [IO.File]::WriteAllText($path+'.next',$body,$utf8)
    Move-Item -LiteralPath ($path+'.next') -Destination $path -Force
}
function Find-Window([string]$Class) {
    @([FloatingR2]::Windows($child.Id) | Where-Object { [FloatingR2]::Class($_) -eq $Class }) | Select-Object -First 1
}
function Wait-Window([string]$Class,[bool]$Present=$true) {
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $h = Find-Window $Class
        if (($null -ne $h) -eq $Present) { return $h }
        if ($child.HasExited) { throw "Owned listener exited: $($child.ExitCode)" }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Window state deadline: $Class present=$Present"
}
function Check-Foreground {
    $h = [FloatingR2]::GetForegroundWindow()
    [uint32]$owner = 0
    [void][FloatingR2]::GetWindowThreadProcessId($h,[ref]$owner)
    if ($owner -eq $child.Id) { throw 'Owned process took foreground focus.' }
    $script:foregroundChecks++
}
function Sample([string]$Phase) {
    $samples = @()
    1..3 | ForEach-Object {
        Start-Sleep -Seconds 3
        Check-Foreground
        $child.Refresh()
        $samples += [ordered]@{
            working_set_mb=[Math]::Round($child.WorkingSet64/1000000,3)
            private_bytes_mb=[Math]::Round($child.PrivateMemorySize64/1000000,3)
            gdi=[FloatingR2]::GetGuiResources($child.Handle,0)
            user=[FloatingR2]::GetGuiResources($child.Handle,1)
            handles=$child.HandleCount
            windows=@([FloatingR2]::Windows($child.Id) | ForEach-Object { [FloatingR2]::Class($_) })
        }
    }
    $script:result[$Phase]=$samples
    Write-Output "$Phase $(($samples | ForEach-Object { "$($_.working_set_mb)/$($_.private_bytes_mb)" }) -join ', ') MB working set/private bytes"
}
function Geometry($h) {
    $r = New-Object FloatingR2+Rect
    if (![FloatingR2]::GetWindowRect($h,[ref]$r)) { throw 'Cannot read own icon bounds.' }
    [ordered]@{left=$r.Left;top=$r.Top;right=$r.Right;bottom=$r.Bottom;width=$r.Right-$r.Left;height=$r.Bottom-$r.Top}
}
function Region($h) {
    $r = [FloatingR2]::CreateRectRgn(0,0,0,0)
    try {
        if ([FloatingR2]::GetWindowRgn($h,$r) -eq 0) { throw 'Icon input region unavailable.' }
        $b = Geometry $h
        $empty = [FloatingR2]::PtInRegion($r,5,[int]($b.height/2))
        $cap = [FloatingR2]::PtInRegion($r,$b.width-10,[int]($b.height/2))
        $atEmpty = [FloatingR2]::WindowFromPoint((New-Object FloatingR2+Point($b.left+5,[int](($b.top+$b.bottom)/2))))
        [ordered]@{left_in_region=$empty;right_cap_in_region=$cap;empty_point_resolves_to_icon=($atEmpty -eq $h)}
    } finally { [void][FloatingR2]::DeleteObject($r) }
}
function Affinity($h) {
    [uint32]$affinity=0
    if (![FloatingR2]::GetWindowDisplayAffinity($h,[ref]$affinity) -or $affinity -ne 17) { throw 'Capture exclusion is not 0x11.' }
    $affinity
}
Write-Snapshot $false $false
$info = New-Object Diagnostics.ProcessStartInfo
$info.FileName = $Exe
$info.WorkingDirectory = $root
$info.UseShellExecute = $false
$info.CreateNoWindow = $true
$info.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
$info.Arguments = "--listen --listener-isolated --floating-icon-replay --data-dir `"$state`" --meetings-dir `"$meetings`" --peers-dir `"$run/peers`" --listener-localappdata `"$run/localappdata`""
$info.EnvironmentVariables['APPDATA'] = Join-Path $run 'appdata'
$info.EnvironmentVariables['LOCALAPPDATA'] = Join-Path $run 'localappdata'
$child = New-Object Diagnostics.Process
$child.StartInfo = $info
$result = [ordered]@{units='MB = 1,000,000 bytes';exe=$Exe;sha256=(Get-FileHash -LiteralPath $Exe -Algorithm SHA256).Hash;isolated=$true;pid=$null;foreground_checks=0}
$foregroundChecks=0
try {
    if (!$child.Start()) { throw 'Listener launch failed.' }
    $result.pid=$child.Id
    Wait-Window 'SidenoteListenerMeetingIcon' $false | Out-Null
    Sample 'idle'
    Write-Snapshot $true $false
    $icon = Wait-Window 'SidenoteListenerMeetingIcon'
    $result.icon_affinity=Affinity $icon
    $style=[FloatingR2]::GetWindowLongPtrW($icon,-20).ToInt64()
    if (($style -band 0x08080088) -ne 0x08080088) { throw 'Expected topmost/toolwindow/noactivate/layered styles.' }
    $result.icon_ex_style=$style
    $result.bead_region=Region $icon
    if ($result.bead_region.left_in_region -or !$result.bead_region.right_cap_in_region -or $result.bead_region.empty_point_resolves_to_icon) { throw 'Unused canvas did not pass hit lookup through.' }
    Sample 'shown'
    $child.Refresh()
    $cpuStart=$child.TotalProcessorTime.TotalMilliseconds
    $watch=[Diagnostics.Stopwatch]::StartNew()
    1..100 | ForEach-Object { Start-Sleep -Milliseconds 100; Check-Foreground }
    $child.Refresh()
    $result.idle_icon_cpu=[ordered]@{cpu_ms=$child.TotalProcessorTime.TotalMilliseconds-$cpuStart;wall_ms=$watch.Elapsed.TotalMilliseconds;percent_one_core=100*($child.TotalProcessorTime.TotalMilliseconds-$cpuStart)/$watch.Elapsed.TotalMilliseconds}
    $geometry=@()
    1..20 | ForEach-Object {
        $cycle=$_
        foreach ($speaking in @($true,$false)) {
            Write-Snapshot $true $speaking
            1..25 | ForEach-Object {
                Start-Sleep -Milliseconds 15
                Check-Foreground
                $g=Geometry $icon
                $g.cycle=$cycle
                $g.speaking_requested=$speaking
                $geometry += $g
            }
            $region=Region $icon
            if ($region.left_in_region -ne $speaking) { throw 'Speech shape did not reach expected input region.' }
        }
    }
    $rights=@($geometry | ForEach-Object {$_.right} | Sort-Object -Unique)
    $widths=@($geometry | ForEach-Object {$_.width} | Sort-Object -Unique)
    if ($rights.Count -ne 1 -or $widths.Count -ne 1) { throw 'Fixed canvas drifted or resized.' }
    $result.cycles=20
    $result.geometry_samples=$geometry.Count
    $result.right_edges=$rights
    $result.right_edge_range_px=$rights[-1]-$rights[0]
    $result.widths=$widths
    $geometry | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $run 'geometry.json') -Encoding UTF8
    # WM_APP dispatches the same card action without mouse/keyboard messages or capture.
    [FloatingR2]::Message($icon,$child.Id,0x8028)
    $card=Wait-Window 'SidenoteListenerMeetingCard'
    Check-Foreground
    $result.card_affinity=Affinity $card
    $result.card_bounds=Geometry $card
    $bounds=Geometry $card
    $bitmap=New-Object Drawing.Bitmap($bounds.width,$bounds.height)
    $graphics=[Drawing.Graphics]::FromImage($bitmap)
    $dc=$graphics.GetHdc()
    try { $result.card_rendered=[FloatingR2]::PrintWindow($card,$dc,2) }
    finally { $graphics.ReleaseHdc($dc);$graphics.Dispose() }
    $bitmap.Save((Join-Path $run 'floating-icon-card.png'),[Drawing.Imaging.ImageFormat]::Png)
    $bitmap.Dispose()
    [FloatingR2]::Message($card,$child.Id,0x10)
    if (!(Find-Window 'SidenoteListenerMeetingIcon')) { throw 'Closing recording card removed icon.' }
    Write-Snapshot $false $false
    Start-Sleep -Milliseconds 500
    if (!(Find-Window 'SidenoteListenerMeetingIcon')) { throw 'Stopped meeting vanished before draft was seen.' }
    $draft=Join-Path $local 'draft-summaries'
    New-Item -ItemType Directory -Force -Path $draft | Out-Null
    [IO.File]::WriteAllText((Join-Path $draft '1794440000000-1.json'),'{"title":"Synthetic review","context":["The team agreed to review the sample tomorrow."],"topics":[],"todo":[],"to_settle":[]}',$utf8)
    Start-Sleep -Milliseconds 500
    if (!(Find-Window 'SidenoteListenerMeetingIcon')) { throw 'Unseen draft removed icon.' }
    [FloatingR2]::Message($icon,$child.Id,0x8028)
    Check-Foreground
    Start-Sleep -Milliseconds 200
    [FloatingR2]::Message($card,$child.Id,0x10)
    Wait-Window 'SidenoteListenerMeetingIcon' $false | Out-Null
    Wait-Window 'SidenoteListenerMeetingCard' $false | Out-Null
    $result.pending_after_ack=Get-Content -Raw -LiteralPath (Join-Path $local 'floating-meetings.json') | ConvertFrom-Json
    Sample 'torn_down'
    $result.gui_resources_returned_to_idle = ($result.torn_down[-1].gdi -eq $result.idle[-1].gdi -and $result.torn_down[-1].user -eq $result.idle[-1].user)
    if (!$result.gui_resources_returned_to_idle) { throw 'Native window resources did not return to their idle counts.' }
    # A second meeting proves classes and resources can be created again after teardown.
    Write-Snapshot $true $false 1 '1794440000000-2'
    $second=Wait-Window 'SidenoteListenerMeetingIcon'
    $result.second_icon_affinity=Affinity $second
    Check-Foreground
    $result.foreground_checks=$foregroundChecks
    $result.completed_utc=[DateTime]::UtcNow.ToString('o')
    $result | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $run 'measurements.json') -Encoding UTF8
    Write-Output "Evidence: $run/measurements.json"
} finally {
    if (!$child.HasExited) {
        [IO.File]::WriteAllText((Join-Path $local 'floating-icon-exit'),'exit',$utf8)
        if (!$child.WaitForExit(10000)) { $child.Kill();$child.WaitForExit();throw 'Owned replay did not exit through its marker.' }
    }
    $child.Dispose()
}

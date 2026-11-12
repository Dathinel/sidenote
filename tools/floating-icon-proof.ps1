param(
    [string]$Exe = "$PSScriptRoot/../target/x86_64-pc-windows-gnu/debug/meeting-icon-spike.exe",
    [string]$Output = "$PSScriptRoot/../artifacts/floating-icon-spike"
)
$ErrorActionPreference = 'Stop'
$Output = [IO.Path]::GetFullPath($Output)
New-Item -ItemType Directory -Force -Path $Output | Out-Null
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class IconProof {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; }
    [StructLayout(LayoutKind.Sequential)] public struct MonitorInfo { public uint Size; public Rect Monitor,Work; public uint Flags; }
    [StructLayout(LayoutKind.Sequential)] public struct NotifyId { public uint Size; public IntPtr Window; public uint Id; public Guid Guid; }
    public delegate bool EnumProc(IntPtr h, IntPtr p);
    public delegate bool MonitorProc(IntPtr monitor,IntPtr dc,ref Rect bounds,IntPtr data);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr data);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h,StringBuilder text,int count);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out Rect r);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out Rect r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref Point p);
    [DllImport("user32.dll")] public static extern IntPtr GetWindowLongPtrW(IntPtr h, int i);
    [DllImport("user32.dll")] public static extern bool GetWindowDisplayAffinity(IntPtr h, out uint affinity);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int width,int height,uint flags);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point p);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint data, UIntPtr extra);
    [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll")] public static extern uint MapVirtualKeyW(uint key, uint mode);
    [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int key);
    [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr h, uint m, UIntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, UIntPtr w, IntPtr l);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowExW(IntPtr parent,IntPtr after,string cls,string name);
    [DllImport("shell32.dll")] public static extern int Shell_NotifyIconGetRect(ref NotifyId id,out Rect rect);
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr c);
    [DllImport("user32.dll")] public static extern bool EnumDisplayMonitors(IntPtr dc,IntPtr clip,MonitorProc callback,IntPtr data);
    [DllImport("user32.dll")] public static extern bool GetMonitorInfoW(IntPtr monitor,ref MonitorInfo info);
    [DllImport("shcore.dll")] public static extern int GetDpiForMonitor(IntPtr monitor,int kind,out uint x,out uint y);
    public static MonitorInfo[] Monitors() {
        var result=new List<MonitorInfo>();
        EnumDisplayMonitors(IntPtr.Zero,IntPtr.Zero,(IntPtr m,IntPtr dc,ref Rect r,IntPtr d) => {
            var info=new MonitorInfo{Size=(uint)Marshal.SizeOf(typeof(MonitorInfo))};
            if(!GetMonitorInfoW(m,ref info)) throw new Exception("GetMonitorInfoW");
            result.Add(info); return true;
        },IntPtr.Zero);
        return result.ToArray();
    }
    public static IntPtr[] Windows(uint pid) {
        var result = new List<IntPtr>();
        EnumWindows((h,p) => { uint id; GetWindowThreadProcessId(h,out id); if(id==pid && IsWindowVisible(h)) result.Add(h); return true; },IntPtr.Zero);
        return result.ToArray();
    }
    public static IntPtr Tray(uint pid) {
        var previous=IntPtr.Zero;
        while(true) {
            var h=FindWindowExW(new IntPtr(-3),previous,"RuneTray",null);
            if(h==IntPtr.Zero) return h;
            uint owner; GetWindowThreadProcessId(h,out owner);
            if(owner==pid) return h;
            previous=h;
        }
    }
    public static int TrayRect(IntPtr window) {
        var id=new NotifyId{Size=(uint)Marshal.SizeOf(typeof(NotifyId)),Window=window,Id=1};
        Rect r; return Shell_NotifyIconGetRect(ref id,out r);
    }
    public static Rect Bounds(IntPtr h) { Rect r; if(!GetWindowRect(h,out r)) throw new Exception("GetWindowRect"); return r; }
    public static bool Borderless(IntPtr h) {
        Rect outer=Bounds(h),client; var origin=new Point();
        if(!GetClientRect(h,out client)||!ClientToScreen(h,ref origin)) throw new Exception("client bounds");
        return origin.X==outer.Left && origin.Y==outer.Top && client.Right==outer.Right-outer.Left && client.Bottom==outer.Bottom-outer.Top;
    }
    public static string Title(IntPtr h) { var text=new StringBuilder(256); GetWindowTextW(h,text,256); return text.ToString(); }
    public static uint Affinity(IntPtr h) { uint a; if(!GetWindowDisplayAffinity(h,out a)) throw new Exception("GetWindowDisplayAffinity"); return a; }
    public static void MoveOn(IntPtr h,int x,int y) {
        SetCursorPos(x,y);
        System.Threading.Thread.Sleep(100);
        var p=new Point{X=x,Y=y};
        var hit=WindowFromPoint(p);
        if(hit!=h) {
            // A later topmost window can cover ours. Raise only the probe, without activation.
            SetWindowPos(h,new IntPtr(-1),0,0,0,0,0x13);
            System.Threading.Thread.Sleep(100);
            hit=WindowFromPoint(p);
        }
        if(hit!=h) {
            uint expected,actual; GetWindowThreadProcessId(h,out expected); GetWindowThreadProcessId(hit,out actual);
            throw new Exception("Pointer target is not the spike; refusing input (expected PID "+expected+", hit PID "+actual+", target "+x+","+y+")");
        }
    }
    public static void Click(IntPtr h,int x,int y) {
        MoveOn(h,x,y); mouse_event(2,0,0,0,UIntPtr.Zero); System.Threading.Thread.Sleep(80); mouse_event(4,0,0,0,UIntPtr.Zero);
    }
    public static void KeyOwn(IntPtr h, byte key) {
        if(GetForegroundWindow()!=h) throw new Exception("Refusing keyboard input outside spike panel");
        foreach(var modifier in new[]{16,17,18,91,92})
            if((GetAsyncKeyState(modifier)&0x8000)!=0) throw new Exception("Owner modifier is held; refusing input");
        byte scan=(byte)MapVirtualKeyW(key,0);
        uint extended=key==40 ? 1u : 0u;
        keybd_event(key,scan,extended,UIntPtr.Zero);
        System.Threading.Thread.Sleep(30);
        keybd_event(key,scan,extended|2u,UIntPtr.Zero);
    }
}
'@
[void][IconProof]::SetProcessDpiAwarenessContext([IntPtr](-4))
[IconProof]::Monitors() | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $Output 'monitors.json')
function Assert($condition, $message) { if (!$condition) { throw $message } }
function Wait-Icon($process) {
    for ($i=0; $i -lt 100; $i++) {
        Assert (!$process.HasExited) 'Spike exited before window appeared'
        $windows = @([IconProof]::Windows($process.Id))
        foreach ($window in $windows) {
            if ([IconProof]::Title($window) -eq 'Sidenote floating icon spike') { return $window }
        }
        Start-Sleep -Milliseconds 200
    }
    throw 'Icon did not appear'
}
function Describe($window) {
    $r = [IconProof]::Bounds($window)
    [ordered]@{ x=$r.Left; y=$r.Top; width=$r.Right-$r.Left; height=$r.Bottom-$r.Top;
        dpi=[IconProof]::GetDpiForWindow($window); style=[IconProof]::GetWindowLongPtrW($window,-16).ToInt64();
        exStyle=[IconProof]::GetWindowLongPtrW($window,-20).ToInt64(); affinity=[IconProof]::Affinity($window); borderless=[IconProof]::Borderless($window) }
}
function Drag-Icon([IntPtr]$icon, [int]$direction, [IntPtr]$panel=[IntPtr]::Zero) {
    $r = [IconProof]::Bounds($icon)
    $x = $r.Left+[int](($r.Right-$r.Left)/2)
    $y = $r.Top+[int](($r.Bottom-$r.Top)/2)
    [IconProof]::MoveOn($icon,$x,$y)
    Start-Sleep -Milliseconds 100
    [IconProof]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds 100
    try {
        for ($step=1; $step -le 12; $step++) {
            $destination=$y+$direction*8*$step
            Assert ([IconProof]::SetCursorPos($x,$destination)) 'SetCursorPos failed'
            Start-Sleep -Milliseconds 80
            $actual=[IconProof+Point]::new()
            [void][IconProof]::GetCursorPos([ref]$actual)
            Assert ($actual.Y -eq $destination -and $actual.X -eq $x) 'Pointer moved outside the proof path'
            if($panel -ne [IntPtr]::Zero -and $step -ge 3){Assert (![IconProof]::IsWindowVisible($panel)) 'Panel stayed visible during drag'}
        }
    } finally { [IconProof]::mouse_event(4,0,0,0,[UIntPtr]::Zero) }
    Start-Sleep -Milliseconds 700
}
$runs = @()
$state = Join-Path $Output 'proof-position.txt'
# Isolated proof state only; repeated runs start from the same fraction.
[IO.File]::WriteAllText($state,'0.4')
foreach ($shape in @('bead','pill','hybrid','restart')) {
    $process = $null
    $afterDrag=$null; $afterDown=$null; $panelInfo=$null; $afterKey=$null; $saved=$null; $survivedClose=$null; $afterCloseMemory=$null; $dropPanel=$null
    try {
        $expectedFraction = [double]::Parse([IO.File]::ReadAllText($state),[Globalization.CultureInfo]::InvariantCulture)
        $foreground = [IconProof]::GetForegroundWindow()
        $arguments = @('--state', ('"'+$state+'"'), '--seconds','90')
        if ($shape -eq 'pill') { $arguments += '--pill' }
        if ($shape -eq 'hybrid') { $arguments += '--hybrid' }
        $process = Start-Process -FilePath $Exe -ArgumentList $arguments -PassThru -WindowStyle Hidden `
            -RedirectStandardOutput (Join-Path $Output "$shape.stdout.log") -RedirectStandardError (Join-Path $Output "$shape.stderr.log")
        $icon = Wait-Icon $process
        Start-Sleep -Seconds 10
        $initial = Describe $icon
        $trayWindow=[IconProof]::Tray($process.Id)
        Assert ($trayWindow -ne [IntPtr]::Zero) 'Rune tray window missing'
        $trayRegistered=[IconProof]::TrayRect($trayWindow) -eq 0
        Assert $trayRegistered 'Shell cannot locate the spike tray icon'
        $scale = $initial.dpi/96.0
        $expectedWidth = if ($shape -eq 'pill') { 136 } elseif($shape -eq 'hybrid'){44} else { 48 }
        $expectedHeight = if ($shape -eq 'pill') { 38 } elseif($shape -eq 'hybrid'){44} else { 48 }
        Assert ($initial.width -eq [int]($expectedWidth*$scale) -and $initial.height -eq [int]($expectedHeight*$scale)) 'Wrong icon size'
        $primary = @([IconProof]::Monitors() | Where-Object { $_.Flags -band 1 })[0]
        $travel = ($primary.Work.Bottom-$primary.Work.Top)/$scale-$expectedHeight-16
        $expectedY = $primary.Work.Top+[int][Math]::Round((8+$expectedFraction*$travel)*$scale)
        Assert ([Math]::Abs($initial.y-$expectedY) -le 1) 'Saved position did not restore inside work area'
        Assert ($initial.x+$initial.width -eq $primary.Work.Right-[int](8*$scale)) 'Icon is outside right work-area edge'
        Assert ($initial.affinity -eq 17) 'Icon capture affinity missing'
        Assert (($initial.exStyle -band 0x08000088) -eq 0x08000088) 'Icon needs NOACTIVATE TOOLWINDOW TOPMOST'
        # Winit retains WS_CAPTION and suppresses the non-client area in WM_NCCALCSIZE.
        Assert $initial.borderless 'Icon has a non-client frame'
        $noActivate = [IconProof]::GetForegroundWindow() -eq $foreground
        Assert $noActivate 'Showing icon changed foreground window'
        $process.Refresh()
        $memory = [ordered]@{ workingSetBytes=(Get-Process -Id $process.Id).WorkingSet64; privateBytes=(Get-Process -Id $process.Id).PrivateMemorySize64 }
        if ($shape -ne 'restart') {
            Drag-Icon $icon 1
            $afterDown = Describe $icon
            Assert ($afterDown.y -gt $initial.y+50) 'Physical mouse drag down did not move icon'
            Drag-Icon $icon -1
            $afterDrag = Describe $icon
            Assert ($afterDrag.y -lt $afterDown.y-50) 'Physical mouse drag up did not move icon'
            Assert ($afterDrag.x -eq $initial.x) 'Drag left the right edge'
            Assert ([IconProof]::GetForegroundWindow() -eq $foreground) 'Dragging activated the icon'
            $saved = [IO.File]::ReadAllText($state)
            [IconProof]::Click($icon,($afterDrag.x+[int]($afterDrag.width/2)),($afterDrag.y+[int]($afterDrag.height/2)))
            Start-Sleep -Seconds 2
            $panel = @([IconProof]::Windows($process.Id) | Where-Object { [IconProof]::Title($_) -eq 'Sidenote floating panel spike' }) | Select-Object -First 1
            Assert ($null -ne $panel) 'Click did not show ordinary panel'
            $panelInfo = Describe $panel
            Assert ($panelInfo.affinity -eq 17) 'Panel capture affinity missing'
            Assert (($panelInfo.exStyle -band 8) -eq 8) 'Panel is not topmost'
            Assert $panelInfo.borderless 'Panel has a non-client frame'
            Assert ([IconProof]::GetForegroundWindow() -eq $panel) 'Panel did not take keyboard focus'
            $scale = $panelInfo.dpi/96.0
            [IconProof]::Click($panel,($panelInfo.x+[int](40*$scale)),($panelInfo.y+[int](100*$scale)))
            foreach ($character in 'spike keyboard 123'.ToCharArray()) {
                [IconProof]::KeyOwn($panel,[byte][char]::ToUpperInvariant($character))
            }
            Start-Sleep -Milliseconds 700
            [IconProof]::Click($panel,($panelInfo.x+[int](55*$scale)),($panelInfo.y+[int](156*$scale)))
            [IconProof]::KeyOwn($panel,40)
            Start-Sleep -Milliseconds 700
            $afterKey = Describe $icon
            Assert ($afterKey.y -gt $afterDrag.y) 'Focused Move button arrow key did not move icon'
            $saved = [IO.File]::ReadAllText($state)
            $typed = Get-Content -Raw -LiteralPath (Join-Path $Output "$shape.stdout.log")
            Assert ($typed.Contains('panel render note="spike keyboard 123"')) 'Keyboard text did not reach Rune TextEdit'
            [void][IconProof]::PostMessageW($panel,0x0010,[UIntPtr]::Zero,[IntPtr]::Zero)
            Start-Sleep -Milliseconds 700
            $survivedClose = !$process.HasExited -and [IconProof]::IsWindowVisible($icon) -and ![IconProof]::IsWindowVisible($panel)
            Assert $survivedClose 'Closing panel did not leave icon running'
            $afterCloseMemory = (Get-Process -Id $process.Id).WorkingSet64
            [IconProof]::Click($icon,($afterKey.x+[int]($afterKey.width/2)),($afterKey.y+[int]($afterKey.height/2)))
            Start-Sleep -Milliseconds 700
            Assert ([IconProof]::IsWindowVisible($panel)) 'Panel did not reopen'
            Drag-Icon $icon 1 $panel
            Assert ([IconProof]::IsWindowVisible($panel)) 'Panel did not reopen on drop'
            $dropPanel=Describe $panel
            $dropIcon=Describe $icon
            $expectedDropY=[Math]::Min($dropIcon.y,$primary.Work.Bottom-$dropPanel.height)
            Assert ($dropPanel.y -eq $expectedDropY) 'Panel did not reopen at its clamped drop position'
            $saved=[IO.File]::ReadAllText($state)
            [void][IconProof]::PostMessageW($panel,0x0010,[UIntPtr]::Zero,[IntPtr]::Zero)
        }
        $runs += [ordered]@{ shape=$shape; pid=$process.Id; initial=$initial; idleMemory=$memory; noActivate=$noActivate;
            afterDown=$afterDown; afterDrag=$afterDrag; panel=$panelInfo; afterKey=$afterKey; savedFraction=$saved; survivedClose=$survivedClose;
            afterCloseWorkingSetBytes=$afterCloseMemory;trayRegistered=$trayRegistered;panelAtDrop=$dropPanel }
        # Exercise the process's own Quit button so Rune removes its tray item on drop.
        $quitIcon=Describe $icon
        [IconProof]::Click($icon,($quitIcon.x+[int]($quitIcon.width/2)),($quitIcon.y+[int]($quitIcon.height/2)))
        Start-Sleep -Milliseconds 500
        $quitPanel=@([IconProof]::Windows($process.Id) | Where-Object {[IconProof]::Title($_) -eq 'Sidenote floating panel spike'})[0]
        $quitRect=Describe $quitPanel
        $quitScale=$quitRect.dpi/96.0
        [IconProof]::Click($quitPanel,($quitRect.x+[int](190*$quitScale)),($quitRect.y+[int](204*$quitScale)))
        Assert ($process.WaitForExit(3000)) 'Quit spike did not exit'
        Assert ([IconProof]::TrayRect($trayWindow) -ne 0) 'Tray item stayed registered after quit'
    } finally {
        if ($null -ne $process -and !$process.HasExited) {
            # End only the process launched in this iteration, even when an assertion fails.
            Stop-Process -Id $process.Id
            $process.WaitForExit()
        }
    }
}
$runs | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $Output 'proof.json')
$runs | ConvertTo-Json -Depth 8

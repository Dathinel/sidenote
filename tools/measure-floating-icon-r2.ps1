param([Parameter(Mandatory=$true)][string]$Exe,[string]$Out="$PSScriptRoot/../artifacts/floating-icon-r2-review")
$ErrorActionPreference='Stop'
$root=(Resolve-Path "$PSScriptRoot/..").Path
$Exe=(Resolve-Path -LiteralPath $Exe).Path
$Out=[IO.Path]::GetFullPath($Out)
if (!$Out.StartsWith($root+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) {throw 'Output must stay in this worktree.'}
$run=Join-Path $Out ('run-'+(Get-Date -Format 'yyyyMMdd-HHmmss'))
$state=Join-Path $run 'data';$local=Join-Path $state 'local';$meetings=Join-Path $run 'meetings'
New-Item -ItemType Directory -Force -Path $local,$meetings,(Join-Path $run 'appdata'),(Join-Path $run 'localappdata') | Out-Null
$utf8=[Text.UTF8Encoding]::new($false)
[IO.File]::WriteAllText((Join-Path $state 'config.json'),'{"notes":{"engine":"off"}}',$utf8)
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class FloatingReview {
 public delegate bool Each(IntPtr h,IntPtr data);
 [StructLayout(LayoutKind.Sequential)] public struct Rect {public int Left,Top,Right,Bottom;}
 [StructLayout(LayoutKind.Sequential)] public struct Point {public int X,Y;public Point(int x,int y){X=x;Y=y;}}
 [DllImport("user32.dll")] public static extern bool EnumWindows(Each cb,IntPtr data);
 [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h,Each cb,IntPtr data);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h,StringBuilder s,int n);
 [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h,out Rect r);
 [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern bool GetWindowDisplayAffinity(IntPtr h,out uint value);
 [DllImport("user32.dll")] public static extern IntPtr GetWindowLongPtrW(IntPtr h,int index);
 [DllImport("user32.dll")] public static extern uint GetGuiResources(IntPtr p,uint kind);
 [DllImport("user32.dll")] static extern IntPtr SendMessageTimeoutW(IntPtr h,uint m,IntPtr w,IntPtr l,uint flags,uint timeout,out IntPtr result);
 [DllImport("user32.dll",CharSet=CharSet.Unicode,EntryPoint="SendMessageTimeoutW")] static extern IntPtr ReadText(IntPtr h,uint m,IntPtr n,StringBuilder s,uint flags,uint timeout,out IntPtr result);
 [DllImport("user32.dll")] static extern bool PostMessageW(IntPtr h,uint m,IntPtr w,IntPtr l);
 [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
 [DllImport("user32.dll")] public static extern int GetWindowRgn(IntPtr h,IntPtr r);
 [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
 [DllImport("gdi32.dll")] public static extern IntPtr CreateRectRgn(int a,int b,int c,int d);
 [DllImport("gdi32.dll")] public static extern bool PtInRegion(IntPtr r,int x,int y);
 [DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr o);
 [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr h,uint attr,out int value,uint n);
 public static bool Cloaked(IntPtr h){int value;return DwmGetWindowAttribute(h,14,out value,4)==0&&value!=0;}
 public static string Class(IntPtr h){var b=new StringBuilder(128);GetClassName(h,b,b.Capacity);return b.ToString();}
 public static IntPtr[] Windows(uint owned){var all=new System.Collections.Generic.List<IntPtr>();EnumWindows((h,d)=>{uint p;GetWindowThreadProcessId(h,out p);if(p==owned)all.Add(h);return true;},IntPtr.Zero);return all.ToArray();}
 static void Own(IntPtr h,uint owned){uint p;GetWindowThreadProcessId(h,out p);if(p!=owned||!Class(h).StartsWith("SidenoteListenerMeeting"))throw new Exception("Refusing access to an unowned HWND");}
 public static int Children(IntPtr h,uint owned){Own(h,owned);int n=0;EnumChildWindows(h,(c,d)=>{n++;return true;},IntPtr.Zero);return n;}
 public static long Message(IntPtr h,uint owned,uint msg){return Message(h,owned,msg,0,0);}
 public static long Message(IntPtr h,uint owned,uint msg,long w){return Message(h,owned,msg,w,0);}
 public static long Message(IntPtr h,uint owned,uint msg,long w,long l){Own(h,owned);IntPtr result;if(SendMessageTimeoutW(h,msg,(IntPtr)w,(IntPtr)l,2,2000,out result)==IntPtr.Zero)throw new Exception("Owned message timeout");return result.ToInt64();}
 public static void Post(IntPtr h,uint owned,uint msg){Own(h,owned);if(!PostMessageW(h,msg,IntPtr.Zero,IntPtr.Zero))throw new Exception("Owned post failed");}
 public static string Text(IntPtr h,uint owned){Own(h,owned);var text=new StringBuilder(8192);IntPtr result;if(ReadText(h,13,(IntPtr)text.Capacity,text,2,2000,out result)==IntPtr.Zero)throw new Exception("Owned text read timeout");return text.ToString();}
}
'@
[void][FloatingReview]::SetThreadDpiAwarenessContext([IntPtr](-4))
function Snapshot([string]$Id,[bool]$Recording,[bool]$Speaking=$false,[bool]$Seen=$false,$Now=$null) {
 $body=@{id=$Id;recording=$Recording;speaking=$Speaking;elapsed=754;seen=$Seen;now_ms=$Now}|ConvertTo-Json -Compress
 $path=Join-Path $local 'floating-icon-replay.json'
 [IO.File]::WriteAllText($path+'.next',$body,$utf8);Move-Item -LiteralPath ($path+'.next') -Destination $path -Force
}
function Draft([string]$Id) {
 $dir=Join-Path $local 'draft-summaries';New-Item -ItemType Directory -Force -Path $dir|Out-Null
 [IO.File]::WriteAllText((Join-Path $dir ($Id+'.json')),'{"title":"Synthetic review","context":["Synthetic notes for the owned-window probe."],"topics":[],"todo":[],"to_settle":[]}',$utf8)
}
function Rolling([string]$Id,[int]$Revision) {
 $dir=Join-Path $meetings $Id;New-Item -ItemType Directory -Force -Path $dir|Out-Null
 $lines=@(@{covered_ms=1000*$Revision}|ConvertTo-Json -Compress)
 1..12|ForEach-Object {$lines+=(@{source='microphone';start_ms=$_*1000;text="Synthetic revision $Revision line $_. "+('sample words '*25)}|ConvertTo-Json -Compress)}
 [IO.File]::WriteAllText((Join-Path $dir 'transcript.rolling.ndjson'),($lines -join "`n")+"`n",$utf8)
}
function Wait-For([scriptblock]$Condition,[string]$Reason,[int]$Seconds=10) {
 $until=[DateTime]::UtcNow.AddSeconds($Seconds)
 do {if (& $Condition) {return};if($child.HasExited){throw 'Owned listener exited'};Start-Sleep -Milliseconds 50}while([DateTime]::UtcNow-lt$until)
 throw "Deadline: $Reason"
}
function Windows([string]$Kind,[bool]$Visible=$false) {
 @([FloatingReview]::Windows($child.Id)|Where-Object{[FloatingReview]::Class($_).StartsWith('SidenoteListenerMeeting'+$Kind+'-') -and (!$Visible -or ([FloatingReview]::IsWindowVisible($_) -and ![FloatingReview]::Cloaked($_)))})
}
function Window([string]$Kind) {Windows $Kind $true|Select-Object -First 1}
function Open-Card($Icon) {
 [void][FloatingReview]::Message($Icon,$child.Id,0x8028)
 Wait-For { $null -ne (Window 'Card') } 'card opens without activation'
 Window 'Card'
}
function Foreground {
 [uint32]$owner=0;[void][FloatingReview]::GetWindowThreadProcessId([FloatingReview]::GetForegroundWindow(),[ref]$owner)
 if($owner-eq$child.Id){throw 'Owned process took foreground focus'}
 if(@(Windows 'Icon' $true).Count-gt1){throw 'Two visible icons'}
 $script:foregroundChecks++
}
function Geometry($h) {
 $r=[FloatingReview+Rect]::new();if(![FloatingReview]::GetWindowRect($h,[ref]$r)){throw 'Owned bounds unavailable'}
 [ordered]@{left=$r.Left;top=$r.Top;right=$r.Right;bottom=$r.Bottom;width=$r.Right-$r.Left;height=$r.Bottom-$r.Top}
}
function Region($h) {
 $r=[FloatingReview]::CreateRectRgn(0,0,0,0)
 try {
  if([FloatingReview]::GetWindowRgn($h,$r)-eq0){throw 'Input region unavailable'}
  $b=Geometry $h
  [ordered]@{left=[FloatingReview]::PtInRegion($r,5,[int]($b.height/2));cap=[FloatingReview]::PtInRegion($r,$b.width-10,[int]($b.height/2));empty_resolves_to_icon=([FloatingReview]::WindowFromPoint([FloatingReview+Point]::new([int]($b.left+5),[int](($b.top+$b.bottom)/2)))-eq$h)}
 }finally{[void][FloatingReview]::DeleteObject($r)}
}
function Affinity($h){[uint32]$a=0;if(![FloatingReview]::GetWindowDisplayAffinity($h,[ref]$a)-or$a-ne17){throw 'Capture exclusion changed'};17}
function Sample([string]$Name) {
 $all=@();1..3|ForEach-Object{
  Start-Sleep -Seconds 3;Foreground;$child.Refresh()
  $all += [ordered]@{working_set_mb=[Math]::Round($child.WorkingSet64/1000000,3);private_bytes_mb=[Math]::Round($child.PrivateMemorySize64/1000000,3);gdi=[FloatingReview]::GetGuiResources($child.Handle,0);user=[FloatingReview]::GetGuiResources($child.Handle,1);handles=$child.HandleCount;windows=@([FloatingReview]::Windows($child.Id)|ForEach-Object{[FloatingReview]::Class($_)})}
 }
 $script:result[$Name]=$all
 $script:result.foreground_checks=$script:foregroundChecks
 $script:result|ConvertTo-Json -Depth 14|Set-Content -LiteralPath (Join-Path $run 'measurements.json') -Encoding UTF8
 Write-Output "$Name $(($all|ForEach-Object{"$($_.working_set_mb)/$($_.private_bytes_mb) GDI=$($_.gdi)"})-join ', ') MB WS/private"
}
function Privacy($h){
 if([FloatingReview]::Children($h,$child.Id)-ne0-or[FloatingReview]::Text($h,$child.Id).Length-ne0-or[FloatingReview]::Message($h,$child.Id,14)-ne0-or[FloatingReview]::Message($h,$child.Id,0x3d,0,-4)-ne0){throw 'Card exposes native text/provider'}
}
function Finish-Meeting([string]$Id) {
 Draft $Id;Snapshot $Id $false $false $true
 Wait-For { @(Windows 'Icon').Count-eq0 -and @(Windows 'Card').Count-eq0 } 'both windows torn down'
 Foreground
}
function Log-Contains([string]$Code){$path=Join-Path $state 'sidenote.log';(Test-Path -LiteralPath $path)-and([IO.File]::ReadAllText($path).Contains($Code))}

Snapshot '' $false
$info=[Diagnostics.ProcessStartInfo]::new();$info.FileName=$Exe;$info.WorkingDirectory=$root;$info.UseShellExecute=$false;$info.CreateNoWindow=$true;$info.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
$info.Arguments="--listen --listener-isolated --floating-icon-replay --data-dir `"$state`" --meetings-dir `"$meetings`" --peers-dir `"$run/peers`" --listener-localappdata `"$run/localappdata`""
$info.EnvironmentVariables['APPDATA']=Join-Path $run 'appdata';$info.EnvironmentVariables['LOCALAPPDATA']=Join-Path $run 'localappdata'
$child=[Diagnostics.Process]::new();$child.StartInfo=$info
$result=[ordered]@{units='MB = 1,000,000 bytes';exe=$Exe;sha256=(Get-FileHash -LiteralPath $Exe -Algorithm SHA256).Hash;foreground_checks=0;show_teardown_cycles=5}
$foregroundChecks=0
try {
 if(!$child.Start()){throw 'Launch failed'};$result.pid=$child.Id
 Sample 'idle'
 $id='1794440000000-1';Rolling $id 1;Snapshot $id $true
 Wait-For {$null-ne(Window 'Icon')} 'icon shows';$icon=Window 'Icon'
 $result.icon_affinity=Affinity $icon;$result.icon_bounds=Geometry $icon
 $region=Region $icon;if($region.left-or!$region.cap-or$region.empty_resolves_to_icon){throw 'Bead click-through failed'};$result.bead_region=$region
 Sample 'shown'
 $child.Refresh();$cpu=$child.TotalProcessorTime.TotalMilliseconds;$watch=[Diagnostics.Stopwatch]::StartNew()
 1..100|ForEach-Object{Start-Sleep -Milliseconds 100;Foreground};$child.Refresh()
 $result.idle_cpu=@{cpu_ms=$child.TotalProcessorTime.TotalMilliseconds-$cpu;wall_ms=$watch.Elapsed.TotalMilliseconds;percent_one_core=100*($child.TotalProcessorTime.TotalMilliseconds-$cpu)/$watch.Elapsed.TotalMilliseconds}
 $geometry=@()
 1..20|ForEach-Object{$cycle=$_;foreach($speech in @($true,$false)){
  Snapshot $id $true $speech
  1..25|ForEach-Object{Start-Sleep -Milliseconds 15;Foreground;$g=Geometry $icon;$g.cycle=$cycle;$geometry+=$g}
  if((Region $icon).left-ne$speech){throw 'Shape endpoint not reached'}
 }}
 $edges=@($geometry|ForEach-Object{$_.right}|Sort-Object -Unique);$sizes=@($geometry|ForEach-Object{"$($_.width)x$($_.height)"}|Sort-Object -Unique)
 if($edges.Count-ne1-or$sizes.Count-ne1){throw 'Drift or resize'}
 $result.drift=@{cycles=20;samples=$geometry.Count;right_edges=$edges;range_px=$edges[-1]-$edges[0];sizes=$sizes}
 $geometry|ConvertTo-Json -Depth 5|Set-Content -LiteralPath (Join-Path $run 'geometry.json') -Encoding UTF8

 $card=Open-Card $icon;Foreground;Privacy $card;$result.card_affinity=Affinity $card
 Wait-For {[FloatingReview]::Message($card,$child.Id,0x8029,3)-gt0} 'scroll range'
 $max=[FloatingReview]::Message($card,$child.Id,0x8029,3)
 $scroll=@();foreach($action in @(5,0,4,1,3,2,7,6,5,4)){$scroll+=[FloatingReview]::Message($card,$child.Id,0x802a,$action);Foreground}
 if($scroll[0]-ne$max-or$scroll[1]-ge$max-or$scroll[2]-ne0-or$scroll[3]-le0-or$scroll[-1]-ne0){throw 'Scroll clamp/navigation failed'}
 $result.scroll=@{max=$max;positions=$scroll;method='private actions sharing the keyboard/wheel handlers; no input messages'}
 $before=[FloatingReview]::Message($card,$child.Id,0x8029,0)
 $beforeImage=(Get-FileHash -LiteralPath (Join-Path $local 'floating-icon-card.png') -Algorithm SHA256).Hash
 2..7|ForEach-Object{Rolling $id $_;Start-Sleep -Milliseconds 150;Foreground}
 Wait-For {[FloatingReview]::Message($card,$child.Id,0x8029,0)-gt$before} 'live text refresh'
 Start-Sleep -Milliseconds 1200
 $gap=[FloatingReview]::Message($card,$child.Id,0x8029,5)
 $afterImage=(Get-FileHash -LiteralPath (Join-Path $local 'floating-icon-card.png') -Algorithm SHA256).Hash
 if($gap-lt1000-or$beforeImage-eq$afterImage){throw 'Live refresh stale or unthrottled'}
 $result.live_refresh=@{minimum_gap_ms=$gap;render_changed=($beforeImage-ne$afterImage)}
 [void][FloatingReview]::Message($card,$child.Id,0x112,0xf060)
 if([FloatingReview]::IsWindowVisible($card)-or[FloatingReview]::Message($card,$child.Id,0x8029,1)-ne0){throw 'SC_CLOSE did not hide and clear'};Privacy $card
 $card=Open-Card $icon
 [void][FloatingReview]::Message($card,$child.Id,0x802b)
 Wait-For {![FloatingReview]::IsWindow($card)} 'actual NCDESTROY'
 $replacement=Open-Card $icon;Privacy $replacement
 $result.close_reentrancy=@{system_close_hides=$true;hidden_content_cleared=$true;destroy_reopen=($replacement-ne$card)}
 Draft $id;Snapshot $id $false;Start-Sleep -Milliseconds 500
 $card=Open-Card $icon;[void][FloatingReview]::Message($card,$child.Id,0x112,0xf060);Start-Sleep -Milliseconds 200
 if(Test-Path -LiteralPath (Join-Path $local "floating-icon-seen/$id.json")){throw 'Title-bar close acknowledged draft'}
 $result.close_reentrancy.title_close_does_not_ack=$true
 Finish-Meeting $id;Sample 'teardown_1'

 2..5|ForEach-Object{
  $cycle=$_;$id="1794440000000-$cycle";Rolling $id 1;Snapshot $id $true
  Wait-For {$null-ne(Window 'Icon')} 'repeat icon';$card=Open-Card (Window 'Icon');Privacy $card;Foreground
  Finish-Meeting $id;Sample "teardown_$cycle"
 }
 $after=@(1..5|ForEach-Object{$result["teardown_$_"][-1]})
 $result.repeated=@{gdi=@($after|ForEach-Object{$_.gdi});user=@($after|ForEach-Object{$_.user});working_set_mb=@($after|ForEach-Object{$_.working_set_mb});private_bytes_mb=@($after|ForEach-Object{$_.private_bytes_mb});gdi_stable=(@($after|ForEach-Object{$_.gdi}|Sort-Object -Unique).Count-eq1);working_set_delta_mb=$after[-1].working_set_mb-$after[0].working_set_mb;private_delta_mb=$after[-1].private_bytes_mb-$after[0].private_bytes_mb}

 $id='1794440000000-20';Snapshot $id $true;Wait-For {$null-ne(Window 'Icon')} 'short-recording icon';$shortIcon=Window 'Icon'
 $dir=Join-Path $meetings $id;New-Item -ItemType Directory -Force -Path $dir|Out-Null
 [IO.File]::WriteAllText((Join-Path $dir 'manifest.json'),'{"status":"complete","duration_ms":10000}',$utf8)
 [IO.File]::WriteAllText((Join-Path $dir 'transcript.ndjson'),'{"text":"Short synthetic meeting"}',$utf8)
 Snapshot $id $false;Wait-For {[FloatingReview]::Message($shortIcon,$child.Id,0x8029,6)-eq1} 'Skipped notice'
 $skipWatch=[Diagnostics.Stopwatch]::StartNew();Wait-For {@(Windows 'Icon').Count-eq0} 'Skipped ends' 6
 if($skipWatch.Elapsed.TotalSeconds-lt2){throw 'Skipped notice too short'}
 $result.skipped=@{shown=$true;visible_seconds=$skipWatch.Elapsed.TotalSeconds}
 $id='1794440000000-21';$now=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds();Snapshot $id $true $false $false $now
 Wait-For {$null-ne(Window 'Icon')} 'no-draft icon';Snapshot $id $false $false $false ($now+1000)
 Wait-For { $p=Get-Content -Raw -LiteralPath (Join-Path $local 'floating-meetings.json')|ConvertFrom-Json; $p.$id.stopped_at_ms -eq ($now+1000) } 'persisted stop deadline'
 Snapshot $id $false $false $false ($now+600999);Start-Sleep -Milliseconds 300
 if($null-eq(Window 'Icon')){throw 'No-draft bound ended early'}
 Snapshot $id $false $false $false ($now+601000);Wait-For {@(Windows 'Icon').Count-eq0} 'no-draft bound'
 $result.no_draft=@{bound_seconds=600;expired=$true;clock='isolated replay clock; production uses persisted UTC stop time'}

 $id='1794440000000-30';Snapshot $id $true;Wait-For {$null-ne(Window 'Icon')} 'panic icon';$old=Window 'Icon'
 [void][FloatingReview]::Message($old,$child.Id,0x802e);Wait-For {(Log-Contains 'FI_ICON_CALLBACK') -and ($null-ne(Window 'Icon')) -and ((Window 'Icon')-ne$old)} 'icon callback containment'
 $card=Open-Card (Window 'Icon');[void][FloatingReview]::Message($card,$child.Id,0x802e)
 Wait-For {(Log-Contains 'FI_CARD_CALLBACK') -and ($null-ne(Window 'Icon'))} 'card callback containment'
 $result.callback_panics=@{icon_contained=$true;card_contained=$true;listener_alive=(!$child.HasExited)}
 $old=Window 'Icon';[FloatingReview]::Post($old,$child.Id,0x802d)
 Wait-For {Test-Path -LiteralPath (Join-Path $local 'floating-icon-stall-started')} 'owned GUI stall'
 Draft $id;Snapshot $id $false $false $true
 Wait-For {Log-Contains 'FI_TEARDOWN_TIMEOUT'} 'bounded teardown timeout' 4
 if(![FloatingReview]::Cloaked($old)){throw 'Retiring icon not cloaked'}
 $next='1794440000000-31';$restart=[Diagnostics.Stopwatch]::StartNew();Snapshot $next $true
 Wait-For {($null-ne(Window 'Icon')) -and ((Window 'Icon')-ne$old)} 'next icon after timeout' 3
 $result.teardown_timeout=@{bound_ms=2000;next_icon_ms=$restart.Elapsed.TotalMilliseconds;retiring_cloaked=[FloatingReview]::Cloaked($old);visible_icons=@(Windows 'Icon' $true).Count}
 Foreground;Wait-For {![FloatingReview]::IsWindow($old)} 'retired GUI exits' 7
 Finish-Meeting $next;Sample 'after_resilience'
 foreach($code in @('FI_CLOAK_FAILED','FI_FONT_RELEASE','FI_BITMAP_RELEASE','FI_DC_RELEASE','FI_DESTROY_FAILED','FI_WORKER_PANIC')){if(Log-Contains $code){throw "Native failure: $code"}}
 $result.pending_after=Get-Content -Raw -LiteralPath (Join-Path $local 'floating-meetings.json')|ConvertFrom-Json
 $result.foreground_checks=$foregroundChecks;$result.completed_utc=[DateTime]::UtcNow.ToString('o')
 $result.cold_resources_returned=($result.teardown_5[-1].gdi-eq$result.idle[-1].gdi-and$result.teardown_5[-1].user-eq$result.idle[-1].user)
 $result.functional_probes_passed=$true
 $result|ConvertTo-Json -Depth 14|Set-Content -LiteralPath (Join-Path $run 'measurements.json') -Encoding UTF8
 Write-Output "Evidence: $run/measurements.json"
 if(!$result.cold_resources_returned){throw 'Cold resource baseline not restored; all measurements retained.'}
}finally{
 if(!$child.HasExited){[IO.File]::WriteAllText((Join-Path $local 'floating-icon-exit'),'exit',$utf8);if(!$child.WaitForExit(10000)){$child.Kill();$child.WaitForExit();throw 'Owned replay did not exit'}}
 $child.Dispose()
}

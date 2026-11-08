param([switch]$Unregister)
$ErrorActionPreference = 'Stop'
$taskName = 'Hex meetings daily check'
if ($Unregister) {
    Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
    Write-Output 'Daily check task unregistered. Reports and memory were kept.'
    return
}
$exe = Join-Path $env:LOCALAPPDATA 'Programs/hex-meetings/hex-meetings.exe'
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "Meetings executable is missing: $exe" }
$identity = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
$action = New-ScheduledTaskAction -Execute $exe -Argument '--daily-check'
$dailyTrigger = New-ScheduledTaskTrigger -Daily -At '08:30'
$logonTrigger = New-ScheduledTaskTrigger -AtLogOn -User $identity
$logonTrigger.Delay = 'PT2M'
$settings = New-ScheduledTaskSettingsSet -StartWhenAvailable -Hidden -MultipleInstances IgnoreNew -ExecutionTimeLimit (New-TimeSpan -Minutes 20) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
$principal = New-ScheduledTaskPrincipal -UserId $identity -LogonType Interactive -RunLevel Limited
Register-ScheduledTask -TaskName $taskName -Action $action -Trigger @($dailyTrigger, $logonTrigger) -Settings $settings -Principal $principal -Description 'Read-only daily alignment check and local company memory.' -Force | Out-Null
Write-Output 'Daily check registered at 08:30 and 2 minutes after user logon, with catch-up after sleep. Existing reports are skipped.'

<#
.SYNOPSIS
    Stop anything from this folder that is playing audio.

.DESCRIPTION
    A panic button. If a voice is coming out of your speakers and you cannot find
    the window, run this.

        .\silence.ps1            show what would be killed
        .\silence.ps1 -Kill      kill it

    It only ever touches processes whose command line references this folder, the
    fixture, or a .wav file -- never an unrelated shell, and never this one.

    play_fixture.py plays in-process via winsound precisely so this script should
    have nothing to do: audio owned by the script stops when the script does.
    The earlier version spawned a PowerShell child to play, and a force-kill of
    the parent left the child talking with no window to explain it. This exists
    because that happened once, and because being unable to find the source of a
    voice is worse than the voice.
#>
param([switch]$Kill)

$self = $PID
$here = (Split-Path -Parent $MyInvocation.MyCommand.Path)

$targets = Get-CimInstance Win32_Process | Where-Object {
    $_.ProcessId -ne $self -and
    $_.CommandLine -and
    (
        $_.CommandLine -match 'play_fixture|make_fixture|meeting-test' -or
        $_.CommandLine -match '\.wav' -or
        $_.CommandLine -match 'Media\.SoundPlayer'
    ) -and
    # Never match a shell that is only *searching* for players, including this
    # one -- that false positive already sent one hunt after its own tail.
    $_.CommandLine -notmatch 'silence\.ps1'
}

if (-not $targets) {
    Write-Host "nothing playing." -ForegroundColor Green
    exit 0
}

Write-Host "found $(@($targets).Count) process(es):" -ForegroundColor Yellow
foreach ($target in $targets) {
    $line = $target.CommandLine
    if ($line.Length -gt 120) { $line = $line.Substring(0, 120) + '...' }
    Write-Host "  PID $($target.ProcessId)  $($target.Name)  started $($target.CreationDate)"
    Write-Host "    $line" -ForegroundColor DarkGray
}

if (-not $Kill) {
    Write-Host "`nre-run with -Kill to stop them." -ForegroundColor Cyan
    exit 0
}

foreach ($target in $targets) {
    try {
        Stop-Process -Id $target.ProcessId -Force -ErrorAction Stop
        Write-Host "  killed $($target.ProcessId)" -ForegroundColor Green
    } catch {
        Write-Host "  could not kill $($target.ProcessId): $_" -ForegroundColor Red
    }
}

# winsound playback belongs to a process; killing the process ends it. This only
# matters if a future change reintroduces an out-of-process player.
Add-Type -AssemblyName System.Windows.Forms -ErrorAction SilentlyContinue
Write-Host "done." -ForegroundColor Green

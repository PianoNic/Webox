# Debug helper: starts `webox --sample` with WEBOX_SAMPLE_CALL=<kind> and saves PNGs of its secondary
# windows (call window, calling dock, incoming toast) via PrintWindow. Offline only; never run without --sample.
# usage: pwsh scripts\capture_windows.ps1 -Kind incoming|ringing|connected|dock -Out C:\tmp\shots [-Wait 6]
param([string]$Kind='connected', [string]$Out="$env:TEMP\callwin_shots", [int]$Wait=6, [string]$Exe="$PSScriptRoot\..\target\release\webox.exe")
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System; using System.Runtime.InteropServices; using System.Text; using System.Collections.Generic;
public class W {
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  public static List<IntPtr> Of(uint pid) { var l = new List<IntPtr>(); EnumWindows((h, _) => { uint p; GetWindowThreadProcessId(h, out p); if (p == pid) l.Add(h); return true; }, IntPtr.Zero); return l; }
}
'@
[void][W]::SetProcessDPIAware()
New-Item -ItemType Directory -Force $Out | Out-Null
$env:WEBOX_SAMPLE_CALL = $Kind
$p = Start-Process $Exe -ArgumentList '--sample' -PassThru
Start-Sleep -Seconds $Wait
foreach ($h in [W]::Of($p.Id)) {
  $sb = New-Object System.Text.StringBuilder 256; [void][W]::GetWindowText($h, $sb, 256); $t = $sb.ToString()
  if ($t -notlike 'Webox -*') { continue }
  $r = New-Object W+RECT; [void][W]::GetWindowRect($h, [ref]$r)
  $w = $r.R - $r.L; $hh = $r.B - $r.T; if ($w -lt 10) { continue }
  $bmp = New-Object System.Drawing.Bitmap $w, $hh; $g = [System.Drawing.Graphics]::FromImage($bmp); $dc = $g.GetHdc()
  [void][W]::PrintWindow($h, $dc, 2); $g.ReleaseHdc($dc); $g.Dispose()
  $name = ($t -replace '[^A-Za-z]', '_'); $bmp.Save("$Out\$Kind-$name.png"); $bmp.Dispose(); "saved $Kind-$name.png ${w}x$hh at $($r.L),$($r.T)"
}
Stop-Process -Id $p.Id
Remove-Item Env:WEBOX_SAMPLE_CALL

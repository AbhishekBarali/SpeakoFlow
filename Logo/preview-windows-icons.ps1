param(
    [string]$OutputPath = (Join-Path $PSScriptRoot 'final-v2/preview/windows-icon-scaling.png')
)

# Offscreen diagnostic only: no windows are read, created, or changed.
# Mirrors tao's RGBA -> CreateIcon path and renders with Windows DrawIconEx.
# The Explorer taskbar may use a different compositor; this is not a screenshot.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$drawingReferences = @('System.Drawing.Common', 'System.Drawing.Primitives')
# PowerShell 7.6 / .NET 10 split the drawing interop into separate assemblies.
foreach ($assembly in @('System.Private.Windows.GdiPlus', 'System.Private.Windows.Core')) {
    if (Test-Path (Join-Path $PSHOME ($assembly + '.dll'))) { $drawingReferences += $assembly }
}
Add-Type -ReferencedAssemblies $drawingReferences -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class IconScalingPreview {
    [DllImport("user32.dll", SetLastError=true)]
    static extern IntPtr CreateIcon(IntPtr instance, int width, int height,
        byte planes, byte bits, byte[] andMask, byte[] xorMask);
    [DllImport("user32.dll", SetLastError=true)]
    static extern bool DrawIconEx(IntPtr dc, int x, int y, IntPtr icon,
        int width, int height, uint step, IntPtr brush, uint flags);
    [DllImport("user32.dll")]
    static extern bool DestroyIcon(IntPtr icon);

    static IntPtr LoadTaoIcon(string path) {
        using (var source = new Bitmap(path)) {
            var bits = source.LockBits(new Rectangle(0, 0, source.Width, source.Height),
                ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
            var bgra = new byte[source.Width * source.Height * 4];
            try {
                for (int y = 0; y < source.Height; y++)
                    Marshal.Copy(IntPtr.Add(bits.Scan0, y * bits.Stride), bgra,
                        y * source.Width * 4, source.Width * 4);
            } finally { source.UnlockBits(bits); }
            var mask = new byte[source.Width * source.Height];
            for (int i = 0; i < mask.Length; i++)
                mask[i] = unchecked((byte)(bgra[i * 4 + 3] - 255));
            var icon = CreateIcon(IntPtr.Zero, source.Width, source.Height, 1, 32, mask, bgra);
            if (icon == IntPtr.Zero) throw new InvalidOperationException("CreateIcon: " + Marshal.GetLastWin32Error());
            return icon;
        }
    }

    public static void Render(string directory, string output) {
        int[] sources = { 24, 32, 48, 64 };
        int[] targets = { 16, 20, 24, 30, 32, 36, 40, 48 };
        using (var canvas = new Bitmap(1024, 710, PixelFormat.Format32bppArgb))
        using (var g = Graphics.FromImage(canvas))
        using (var titleFont = new Font("Segoe UI", 15))
        using (var font = new Font("Segoe UI", 10))
        using (var dim = new SolidBrush(Color.FromArgb(170, 170, 170))) {
            g.Clear(Color.FromArgb(32, 32, 32));
            g.DrawString("Windows icon scaling - original artwork", titleFont, Brushes.White, 28, 20);
            g.DrawString("CreateIcon + DrawIconEx, offscreen. Top: actual pixels. Below: 4x nearest-neighbor inspection.", font, dim, 28, 55);
            for (int column = 0; column < targets.Length; column++)
                g.DrawString(targets[column] + " px", font, dim, 170 + column * 105, 94);
            for (int row = 0; row < sources.Length; row++) {
                int top = 124 + row * 134;
                g.DrawString(sources[row] + " px source", font, Brushes.White, 28, top + 18);
                var icon = LoadTaoIcon(System.IO.Path.Combine(directory, "icon-" + sources[row] + ".png"));
                try {
                    for (int column = 0; column < targets.Length; column++) {
                        int size = targets[column], left = 160 + column * 105;
                        using (var tile = new Bitmap(size, size, PixelFormat.Format32bppArgb)) {
                            using (var tileGraphics = Graphics.FromImage(tile)) {
                                tileGraphics.Clear(Color.FromArgb(32, 32, 32));
                                IntPtr dc = tileGraphics.GetHdc();
                                try {
                                    if (!DrawIconEx(dc, 0, 0, icon, size, size, 0, IntPtr.Zero, 3))
                                        throw new InvalidOperationException("DrawIconEx: " + Marshal.GetLastWin32Error());
                                } finally { tileGraphics.ReleaseHdc(dc); }
                            }
                            g.DrawImageUnscaled(tile, left + (80 - size) / 2, top);
                            // Crop the central mark for an equal-sized pixel-level comparison.
                            g.InterpolationMode = System.Drawing.Drawing2D.InterpolationMode.NearestNeighbor;
                            g.PixelOffsetMode = System.Drawing.Drawing2D.PixelOffsetMode.Half;
                            int crop = Math.Min(size, 16);
                            g.DrawImage(tile, new Rectangle(left + 8, top + 48, crop * 4, crop * 4),
                                new Rectangle((size - crop) / 2, (size - crop) / 2, crop, crop), GraphicsUnit.Pixel);
                        }
                    }
                } finally { DestroyIcon(icon); }
            }
            g.DrawString("This checks the native bitmap conversion and scaling path. It does not prove Explorer's live rendering.", font, dim, 28, 678);
            canvas.Save(output, ImageFormat.Png);
        }
    }
}
'@
$outputFile = [System.IO.Path]::GetFullPath($OutputPath)
[IconScalingPreview]::Render((Join-Path $PSScriptRoot 'final-v2/small/png'), $outputFile)
Write-Output "Saved $outputFile"

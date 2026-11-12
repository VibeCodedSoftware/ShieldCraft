ShieldCraft 0.1.0 (Windows x64)
====================================

Run without installing:  double-click ShieldCraft.exe (this folder is fully portable;
                         create an empty file named portable.flag here to keep settings in .\data)
Install for this user:   double-click "Install ShieldCraft.cmd"
Uninstall:               Settings > Apps > Installed apps > ShieldCraft

Requirements: Windows 10/11 64-bit, a graphics driver with OpenGL 2.0+. No other runtimes.
Optional: FFmpeg (ffmpeg.exe on PATH, next to the app, or set in Edit > Preferences) for MP4/WebM.

What works in this build
  * Grid Noise Studio - animated grid-noise protection (GIF, APNG, PNG frames; MP4/WebM with FFmpeg),
    still exports with static layers, storyboards, presets, measured frame-average check.
    Technique by stelmosfire13-alt (Grid Noise Animator), independently implemented.
  * Hope:RE Noise - adversarial noise optimised against a CLIP model (method and model by HopeArtOrg,
    MIT). Needs its 335 MB model, downloaded from the app on request. Runs on the CPU in a separate
    worker process; expect minutes per image.
  * Hope:RE Style Cloak (Glaze mode) - noise that nudges a CLIP model's reading of your style toward a
    decoy style you choose (method and model by HopeArtOrg, MIT; not the University of Chicago Glaze).
    Needs its own 335 MB model, downloaded on request.
  * Hope:RE Concept Poison (Nightshade mode) - noise that makes a CLIP model read the artwork as a
    decoy concept you choose (method and model by HopeArtOrg, MIT; not the University of Chicago
    Nightshade). Needs about 150 iterations per tile; every run reports whether it actually worked.
  * Live preview - Simulated / Pattern overlay while adjusting Hope:RE strength.
  * Auto-iterate (Hope:RE modes, on by default) - tiles whose measured loss has not improved get
    extra rounds automatically, up to a cap; the report says if any area still did not improve.
  * Effect Monitor - histogram + pattern tile showing how each engine changes the image, live.
  * Difference view - amplifies |protected - original| so invisible changes can be checked.
  * Signature Ink - invisible authorship signature, embed and verify (compatible with Hope:RE).
  * Optional selection tools - rectangle (M), ellipse (Shift+M), lasso (L), brush (B) to protect
    only part of an image. Shift adds, Alt subtracts, Ctrl+Shift+I inverts, Ctrl+D deselects.
    With no selection the whole image is protected. Press F1 for all keyboard shortcuts.
  * Batch processing, processing reports, projects, undo/redo, Third-Party Licenses window.

Everything runs locally. Nothing is uploaded unless you confirm an optional Lantern image check.
Model downloads (Hope:RE) happen only after you confirm. No accounts, no telemetry.
No protection method can guarantee artwork will not be used for AI training, copied or imitated.

Command line: shieldcraft-cli.exe (run without arguments for help).
Licenses: THIRD_PARTY_NOTICES.md and the licenses folder.

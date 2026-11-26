// The plugin process: `--describe` prints the descriptor (`--form`: with the form laid out by hand); otherwise it serves.
await Prefs.PrefsPlugin.Define(args.Contains("--form")).Serve();

# Screen text (recovery). Keys are "recovery-" plus a short name from the English text; tooltip keys end in -tip.

recovery-cannot-save-the-recovery-settings = Cannot save the recovery settings.
recovery-cannot-open-the-recovery = Cannot open the recovery
recovery-automatic = Automatic
recovery-low = Low
recovery-standard = Standard
recovery-high = High
recovery-custom = Custom
recovery-untitled-recovered = Untitled (Recovered)
recovery-internal-error-while-writing = Internal error while writing
recovery-not-a-generation-in-the = Not a generation in the recovery folder
recovery-low-disk-space = Low disk space
recovery-recovery-checkpoint-skipped = Recovery checkpoint skipped
recovery-recovery-checkpoint-failed = Recovery checkpoint failed
recovery-recovery-checkpoints-are-working-again = Recovery checkpoints are working again
recovery-recovery-setting-is-out-of = Recovery setting { $key }={ $value } is out of range ({ $range }–{ $range_2 }); using the default
recovery-recovery-folder-is-not-absolute = Recovery folder { $dir } is not absolute; using the default
recovery-generations-are-not-trimmed-because = Generations are not trimmed because the recovery settings cannot be read ({ $reason })
recovery-cannot-settle-the-previous-recovery = Cannot settle the previous recovery marker
recovery-recovery-is-unavailable = Recovery is unavailable
recovery-in-use = { $used } in use
recovery-disk-space-used-by-recovery-tip = Disk space used by recovery
recovery-this-session-tip = This session
recovery-crashed-sessions-tip = Crashed sessions
recovery-closed-sessions-tip = Closed sessions
recovery-other-windows-tip = Other windows
recovery-limit-tip = Limit
recovery-free-on-disk-tip = Free on disk
recovery-just-now = just now
recovery-s-ago = { $s } s ago
recovery-min-ago = { $s } min ago
recovery-h-ago = { $s } h ago
recovery-d-ago = { $s } d ago
recovery-time-from-a-change-to-tip = Time from a change to its checkpoint
recovery-how-much-disk-space-the-tip = How much disk space the recovery generations may use. Beyond it, the oldest generations are removed (the newest of this session and of each crashed session stay). Automatic is 2 GB or 10% of the disk recovery can use, whichever is smaller. When the disk is nearly full, checkpoints are skipped whatever the limit
recovery-number-of-generations-to-keep-tip = Number of generations to keep (older ones are trimmed)
recovery-unreadable = Unreadable
recovery-untitled = Untitled
recovery-recovery = Recovery
recovery-close-window = Close Window
recovery-min = { $n } min
recovery-s = { $n } s
recovery-checkpoint-interval = Checkpoint interval
recovery-generations-kept = Generations kept
recovery-disk-space = Disk space
recovery-details = Details
recovery-the-disk-space-the-recovery-tip = The disk space the recovery generations may use (1 GB = 1024 MB). Releasing it replaces the level with a custom amount and removes the oldest generations if the limit is exceeded
recovery-limit = Limit
recovery-crash-report = Crash Report
recovery-open = Open
recovery-opens-the-selected-generation-as = Opens the selected generation as “Untitled (Recovered)”. The original file is not written
recovery-discard = Discard
recovery-deletes-the-selected-generation-saved = Deletes the selected generation. Saved .ylp files stay
recovery-close = Close
recovery-discard-generation = Discard Generation
recovery-cancel = Cancel
recovery-deletes-this-generation-permanently-saved-tip = Deletes this generation permanently. Saved .ylp files stay
recovery-sets-count = { $count } { $count ->
        [one] set
       *[other] sets
    }
recovery-damaged-generation-cannot-be-read = { $reason ->
       *[other] Damaged generation (cannot be read)
    }

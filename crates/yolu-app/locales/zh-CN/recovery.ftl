# 界面文本（recovery）。键名为“recovery-”加上取自英文的简短名称，工具提示的键名以 -tip 结尾。

recovery-cannot-save-the-recovery-settings = 无法保存恢复设置。
recovery-cannot-open-the-recovery = 无法打开恢复数据
recovery-automatic = 自动
recovery-low = 少
recovery-standard = 标准
recovery-high = 多
recovery-custom = 自定义
recovery-untitled-recovered = 未命名（已恢复）
recovery-internal-error-while-writing = 写入时发生内部错误
recovery-not-a-generation-in-the = 不是恢复文件夹中的恢复点
recovery-low-disk-space = 磁盘空间不足
recovery-recovery-checkpoint-skipped = 已略过恢复存盘
recovery-recovery-checkpoint-failed = 恢复存盘失败
recovery-recovery-checkpoints-are-working-again = 恢复存盘已恢复正常
recovery-recovery-setting-is-out-of = 恢复设置 { $key }={ $value } 超出范围（{ $range }～{ $range_2 }），改用默认值
recovery-recovery-folder-is-not-absolute = 恢复文件夹 { $dir } 不是绝对路径，改用默认值
recovery-generations-are-not-trimmed-because = 无法读取恢复设置，因此不整理恢复点（{ $reason }）
recovery-cannot-settle-the-previous-recovery = 无法清理上次的恢复标记
recovery-recovery-is-unavailable = 无法使用恢复功能
recovery-in-use = 已使用 { $used }
recovery-disk-space-used-by-recovery-tip = 恢复功能使用的磁盘空间
recovery-this-session-tip = 本次会话
recovery-crashed-sessions-tip = 崩溃的会话
recovery-closed-sessions-tip = 已关闭的会话
recovery-other-windows-tip = 其他窗口
recovery-limit-tip = 上限
recovery-free-on-disk-tip = 磁盘可用空间
recovery-just-now = 刚刚
recovery-s-ago = { $s } 秒前
recovery-min-ago = { $s } 分钟前
recovery-h-ago = { $s } 小时前
recovery-d-ago = { $s } 天前
recovery-time-from-a-change-to-tip = 从更改到创建恢复存盘的时间
recovery-how-much-disk-space-the-tip = 恢复点可使用的磁盘空间。超过时，会从最旧的恢复点开始删除（本次运行与每次崩溃运行的最新恢复点会保留）。自动为 2 GB 与恢复功能可用磁盘空间的 10% 两者中较小者。磁盘快满时，无论上限为何都会略过恢复存盘
recovery-number-of-generations-to-keep-tip = 要保留的恢复点数量（超过的会从最旧的开始整理）
recovery-unreadable = 无法读取
recovery-untitled = 未命名
recovery-recovery = 恢复
recovery-close-window = 关闭窗口
recovery-min = { $n } 分钟
recovery-s = { $n } 秒
recovery-checkpoint-interval = 恢复存盘间隔
recovery-generations-kept = 保留的恢复点
recovery-disk-space = 磁盘空间
recovery-details = 详细信息
recovery-the-disk-space-the-recovery-tip = 恢复点可使用的磁盘空间（1 GB = 1024 MB）。拖动后放开会将等级改为自定义的空间大小，若超过上限，会从最旧的恢复点开始删除
recovery-limit = 上限
recovery-crash-report = 崩溃报告
recovery-open = 打开
recovery-opens-the-selected-generation-as = 将所选的恢复点打开为“未命名（已恢复）”。不会写入源文件
recovery-discard = 舍弃
recovery-deletes-the-selected-generation-saved = 删除所选的恢复点。已保存的 .ylp 文件会保留
recovery-close = 关闭
recovery-discard-generation = 舍弃恢复点
recovery-cancel = 取消
recovery-deletes-this-generation-permanently-saved-tip = 永久删除此恢复点。已保存的 .ylp 文件会保留
recovery-sets-count = { $count } 个纹理集
recovery-damaged-generation-cannot-be-read = { $reason ->
       *[other] 恢复点已损坏（无法读取）
    }

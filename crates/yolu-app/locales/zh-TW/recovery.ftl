# 畫面文字（recovery）。鍵名為「recovery-」加上取自英文的簡短名稱，工具提示的鍵名以 -tip 結尾。

recovery-cannot-save-the-recovery-settings = 無法儲存回復設定。
recovery-cannot-open-the-recovery = 無法開啟回復資料
recovery-automatic = 自動
recovery-low = 少
recovery-standard = 標準
recovery-high = 多
recovery-custom = 自訂
recovery-untitled-recovered = 未命名（已回復）
recovery-internal-error-while-writing = 寫入時發生內部錯誤
recovery-not-a-generation-in-the = 不是回復資料夾中的回復點
recovery-low-disk-space = 磁碟空間不足
recovery-recovery-checkpoint-skipped = 已略過回復存檔
recovery-recovery-checkpoint-failed = 回復存檔失敗
recovery-recovery-checkpoints-are-working-again = 回復存檔已恢復正常
recovery-recovery-setting-is-out-of = 回復設定 { $key }={ $value } 超出範圍（{ $range }～{ $range_2 }），改用預設值
recovery-recovery-folder-is-not-absolute = 回復資料夾 { $dir } 不是絕對路徑，改用預設值
recovery-generations-are-not-trimmed-because = 無法讀取回復設定，因此不整理回復點（{ $reason }）
recovery-cannot-settle-the-previous-recovery = 無法清理上次的回復標記
recovery-recovery-is-unavailable = 無法使用回復功能
recovery-in-use = 已使用 { $used }
recovery-disk-space-used-by-recovery-tip = 回復功能使用的磁碟空間
recovery-this-session-tip = 本次工作階段
recovery-crashed-sessions-tip = 當機的工作階段
recovery-closed-sessions-tip = 已關閉的工作階段
recovery-other-windows-tip = 其他視窗
recovery-limit-tip = 上限
recovery-free-on-disk-tip = 磁碟可用空間
recovery-just-now = 剛剛
recovery-s-ago = { $s } 秒前
recovery-min-ago = { $s } 分鐘前
recovery-h-ago = { $s } 小時前
recovery-d-ago = { $s } 天前
recovery-time-from-a-change-to-tip = 從變更到建立回復存檔的時間
recovery-how-much-disk-space-the-tip = 回復點可使用的磁碟空間。超過時，會從最舊的回復點開始刪除（本次執行與每次當機執行的最新回復點會保留）。自動為 2 GB 與回復功能可用磁碟空間的 10% 兩者中較小者。磁碟快滿時，無論上限為何都會略過回復存檔
recovery-number-of-generations-to-keep-tip = 要保留的回復點數量（超過的會從最舊的開始整理）
recovery-unreadable = 無法讀取
recovery-untitled = 未命名
recovery-recovery = 回復
recovery-close-window = 關閉視窗
recovery-min = { $n } 分鐘
recovery-s = { $n } 秒
recovery-checkpoint-interval = 回復存檔間隔
recovery-generations-kept = 保留的回復點
recovery-disk-space = 磁碟空間
recovery-details = 詳細資訊
recovery-the-disk-space-the-recovery-tip = 回復點可使用的磁碟空間（1 GB = 1024 MB）。拖曳後放開會將等級改為自訂的空間大小，若超過上限，會從最舊的回復點開始刪除
recovery-limit = 上限
recovery-crash-report = 當機報告
recovery-open = 開啟
recovery-opens-the-selected-generation-as = 將所選的回復點開啟為「未命名（已回復）」。不會寫入原始檔案
recovery-discard = 捨棄
recovery-deletes-the-selected-generation-saved = 刪除所選的回復點。已儲存的 .ylp 檔案會保留
recovery-close = 關閉
recovery-discard-generation = 捨棄回復點
recovery-cancel = 取消
recovery-deletes-this-generation-permanently-saved-tip = 永久刪除此回復點。已儲存的 .ylp 檔案會保留
recovery-sets-count = { $count } 個紋理集
recovery-damaged-generation-cannot-be-read = { $reason ->
       *[other] 回復點已損壞（無法讀取）
    }

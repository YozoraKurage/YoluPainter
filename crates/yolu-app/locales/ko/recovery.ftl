# 화면 문구(recovery). 키는 "recovery-" 에 영어 문장에서 딴 짧은 이름을 붙인 것이며, 툴팁의 키는 -tip으로 끝납니다.

recovery-cannot-save-the-recovery-settings = 복구 설정을 저장할 수 없습니다.
recovery-cannot-open-the-recovery = 복구를 열 수 없습니다
recovery-automatic = 자동
recovery-low = 낮음
recovery-standard = 표준
recovery-high = 높음
recovery-custom = 사용자 지정
recovery-untitled-recovered = 제목 없음(복구됨)
recovery-internal-error-while-writing = 쓰는 중 내부 오류가 발생했습니다
recovery-not-a-generation-in-the = 복구 폴더의 세대가 아닙니다
recovery-low-disk-space = 디스크 여유 공간이 부족합니다
recovery-recovery-checkpoint-skipped = 복구 체크포인트를 건너뛰었습니다
recovery-recovery-checkpoint-failed = 복구 체크포인트에 실패했습니다
recovery-recovery-checkpoints-are-working-again = 복구 체크포인트가 다시 작동합니다
recovery-recovery-setting-is-out-of = 복구 설정 { $key }={ $value }이(가) 범위({ $range }–{ $range_2 })를 벗어나 기본값을 사용합니다
recovery-recovery-folder-is-not-absolute = 복구 폴더 { $dir }이(가) 절대 경로가 아니어서 기본값을 사용합니다
recovery-generations-are-not-trimmed-because = 복구 설정을 읽을 수 없어 세대를 정리하지 않습니다({ $reason })
recovery-cannot-settle-the-previous-recovery = 이전 복구 표식을 정리할 수 없습니다
recovery-recovery-is-unavailable = 복구를 사용할 수 없습니다
recovery-in-use = { $used } 사용 중
recovery-disk-space-used-by-recovery-tip = 복구가 사용하는 디스크 용량
recovery-this-session-tip = 이번 세션
recovery-crashed-sessions-tip = 충돌한 세션
recovery-closed-sessions-tip = 종료한 세션
recovery-other-windows-tip = 다른 창
recovery-limit-tip = 한도
recovery-free-on-disk-tip = 디스크 여유 공간
recovery-just-now = 방금
recovery-s-ago = { $s }초 전
recovery-min-ago = { $s }분 전
recovery-h-ago = { $s }시간 전
recovery-d-ago = { $s }일 전
recovery-time-from-a-change-to-tip = 변경부터 체크포인트까지의 시간
recovery-how-much-disk-space-the-tip = 복구 세대가 사용할 수 있는 디스크 용량입니다. 넘으면 오래된 세대부터 제거됩니다(이번 세션과 충돌한 각 세션의 최신 세대는 남습니다). 자동은 복구가 쓸 수 있는 디스크의 10%와 2GB 중 작은 쪽입니다. 디스크가 거의 가득 차면 한도와 상관없이 체크포인트를 건너뜁니다
recovery-number-of-generations-to-keep-tip = 보관할 세대의 수(오래된 것은 정리됩니다)
recovery-unreadable = 읽을 수 없음
recovery-untitled = 제목 없음
recovery-recovery = 복구
recovery-close-window = 창 닫기
recovery-min = { $n }분
recovery-s = { $n }초
recovery-checkpoint-interval = 체크포인트 간격
recovery-generations-kept = 보관할 세대
recovery-disk-space = 디스크 용량
recovery-details = 상세
recovery-the-disk-space-the-recovery-tip = 복구 세대가 사용할 수 있는 디스크 용량입니다(1GB = 1024MB). 값을 바꾸면 단계가 사용자 지정 용량으로 바뀌고, 한도를 넘으면 오래된 세대부터 제거됩니다
recovery-limit = 한도
recovery-crash-report = 충돌 보고서
recovery-open = 열기
recovery-opens-the-selected-generation-as = 선택한 세대를 ‘제목 없음(복구됨)’으로 엽니다. 원래 파일에는 쓰지 않습니다
recovery-discard = 버리기
recovery-deletes-the-selected-generation-saved = 선택한 세대를 삭제합니다. 저장된 .ylp 파일은 그대로 남습니다
recovery-close = 닫기
recovery-discard-generation = 세대 버리기
recovery-cancel = 취소
recovery-deletes-this-generation-permanently-saved-tip = 이 세대를 완전히 삭제합니다. 저장된 .ylp 파일은 그대로 남습니다
recovery-sets-count = 세트 { $count }개
recovery-damaged-generation-cannot-be-read = { $reason ->
       *[other] 손상된 세대(읽을 수 없음)
    }

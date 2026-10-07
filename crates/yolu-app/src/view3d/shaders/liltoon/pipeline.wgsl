// 出どころ: lilToon 2.3.4（MIT、Copyright (c) 2020-present lilxyzw）の機能の入切（この部品のパイプラインの定数は 3D ビューのもの）。許諾と出どころの全文は ../THIRD-PARTY-NOTICES.md。
// ───────── パイプラインの定数（既定は全部入り） ─────────

// 機能のビット（F_*）。0 のビットの機能はパイプラインに作らない
override LIL_FEATURES: u32 = 0xffffffffu;
// スロットの読み方のビット（スロットの番号。0〜31 と 32〜63）: 1 つの元をそのまま読む・成分ごとに読む。どちらも 0 のスロットは既定の値
override LIL_SINGLE0: u32 = 0xffffffffu;
override LIL_SINGLE1: u32 = 0xffffffffu;
override LIL_LOOP0: u32 = 0xffffffffu;
override LIL_LOOP1: u32 = 0xffffffffu;

const F_SHADOW: u32 = 0u;
const F_BUMP: u32 = 1u;
const F_BUMP2: u32 = 2u;
const F_MAIN2: u32 = 3u;
const F_MAIN3: u32 = 4u;
const F_ALPHA_MASK: u32 = 5u;
const F_RIM_SHADE: u32 = 6u;
const F_BACKLIGHT: u32 = 7u;
const F_REFLECTION: u32 = 8u;
const F_MATCAP: u32 = 9u;
const F_MATCAP2: u32 = 10u;
const F_RIM: u32 = 11u;
const F_GLITTER: u32 = 12u;
const F_EMISSION: u32 = 13u;
const F_EMISSION2: u32 = 14u;
const F_ANISO: u32 = 15u;
const F_DISTANCE_FADE: u32 = 16u;
const F_BACKFACE: u32 = 17u;

fn feat(bit: u32) -> bool {
    return (LIL_FEATURES & (1u << bit)) != 0u;
}

fn slot_bit(lo: u32, hi: u32, i: i32) -> bool {
    if (i < 32) {
        return (lo & (1u << u32(i))) != 0u;
    }
    return (hi & (1u << u32(i - 32))) != 0u;
}


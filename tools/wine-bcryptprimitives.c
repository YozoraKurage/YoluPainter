/* 古い Wine の試験専用。Windows の配布物へは同梱しない。
 * Rust の ProcessPrng を Wine のシステム乱数へ接続する。固定値にはしない。
 */
#include <windows.h>
#include <limits.h>
BOOLEAN WINAPI SystemFunction036(PVOID buffer, ULONG length);
__declspec(dllexport) BOOL WINAPI ProcessPrng(PBYTE buffer, SIZE_T length) {
    while (length) {
        ULONG part = length > ULONG_MAX ? ULONG_MAX : (ULONG)length;
        if (!SystemFunction036(buffer, part)) return FALSE;
        buffer += part;
        length -= part;
    }
    return TRUE;
}

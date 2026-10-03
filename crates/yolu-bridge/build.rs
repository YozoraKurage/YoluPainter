//! C# の P/Invoke の宣言（generated/LiveLinkNative.g.cs）を src/ffi.rs から作る。Unity 版の Editor/LiveLink/Native へ写して使う。

fn main() {
    println!("cargo:rerun-if-changed=src/ffi.rs");
    println!("cargo:rerun-if-changed=src/testserver.rs");
    csbindgen::Builder::default()
        .input_extern_file("src/ffi.rs")
        .input_extern_file("src/testserver.rs")
        .csharp_dll_name("yolu_bridge")
        .csharp_class_name("LiveLinkNative")
        .csharp_namespace("Yozolab.YoluPainter.Editor.LiveLink")
        .csharp_class_accessibility("internal")
        .csharp_use_nint_types(false)
        .generate_csharp_file("generated/LiveLinkNative.g.cs")
        .expect("C# の宣言を作れない");
}

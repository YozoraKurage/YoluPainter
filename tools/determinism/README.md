# 数学関数の値を採る

C# の Math と Rust の f64 を、同じビット列の入力で比較する小さなツール。`probe.rs` は依存のない Rust example で、crates の実装を変更せず `rustc` でビルドできる。
出力は `識別子 関数 入力x 入力y 結果`。入力は IEEE 754 double の 16 桁 hex、結果は Math/f64 なら 16 桁、MathF/f32 なら 8 桁。
`atan2` の入力順は各 API の引数順（最初が y、次が x）。式を写した `amount` / `azimuth` はペンの x、y の順。

Python 3.10 以降が必要。入力生成は標準ライブラリだけ。生成物とログはリポジトリの `target/determinism/` に置く。

Linux での一括比較:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
tools/wine-tests.sh --compat-bcrypt  # 既知の pen_tilt の失敗はそのまま記録する
python3 tools/determinism/run.py
```

Unity 同梱 .NET・Roslyn・Mono が必要（既定 `/opt/unity/Editor/Data`、`YOLUPAINTER_CORE_UNITY_DATA` で変更）。
`run.py` は Linux Math と f64、Wine の Windows GNU f64 を測る。MathF は以下の明示的な別測定にする。
`rust-wine.tsv` は相違のある入力すべて、`.json` は件数・最大 ULP・異なるビットの個数・最初の入力。`*-stages.*` は傾きの sqrt/atan/atan2 の分離測定。
`environment.json` に Rust・Mono・Wine の版を記録する。NaN のペイロード・符号付きゼロの意味を評価するツールではない。

## Windows 実機（PowerShell と cmd）

1. このリポジトリを Windows に用意し、ルートで入力を生成する。Linux で作った同じ `inputs.txt` を運んでもよい。小数文字列からの再解釈はしない。

   ```powershell
   New-Item -ItemType Directory -Force target/determinism
   python tools/determinism/compare.py --inputs target/determinism/inputs.txt
   Get-Content target/determinism/inputs.txt | Where-Object { $_ -notmatch '^pen:' } | Set-Content -Encoding ascii target/determinism/mathf-inputs.txt
   rustc -Vv > target/determinism/rust-version.txt
   rustc -O tools/determinism/probe.rs -o target/determinism/rust-msvc.exe --target x86_64-pc-windows-msvc
   cmd /c 'target\determinism\rust-msvc.exe < target\determinism\inputs.txt > target\determinism\rust-msvc.txt'
   cmd /c 'target\determinism\rust-msvc.exe --mathf < target\determinism\mathf-inputs.txt > target\determinism\rust-msvc-mathf.txt'
   ```

   MSVC 用の Rust target と Visual C++ のビルドツールが必要。GNU は MinGW と GNU target を用意し、別名の exe とログで同じ手順を行う。
   PowerShell の版によるリダイレクトの UTF-16 化を避けるため、入力・結果のリダイレクトには `cmd /c` を使っている。

2. Windows Unity の **エディタ内 Mono** で採る。自分のデータを入れない検証用の空プロジェクトに `Probe.cs` を置き、エディタのメニュー／試験メソッドから次を呼ぶ。
   入出力先は上で用意した `target/determinism/` の絶対パスにする。`Probe.cs` 自体には Unity の依存がない。

   ```csharp
   using (var input = System.IO.File.OpenText(inputPath))
   using (var output = new System.IO.StreamWriter(outputPath))
       Probe.Run(input, output);
   // MathF は別の入力 mathf-inputs.txt と出力を使う:
   // Probe.RunMathF(input, output);
   ```

   `Application.unityVersion`、`Environment.Version`、Windows の版・CPU・API Compatibility Level・バックエンドを別ログに残す。
   MathF がないランタイムでは `NotSupportedException` で止まる。Math へ黙って代用しない。
   `Mathf` とネイティブ `Quaternion.Euler` の検証はこのツールに含まれず、カメラの実機照合で別に採る。

3. Unity に同梱された Mono を単体で起動できる場合は、同梱 Roslyn で `Probe.cs` を exe にビルドする。
   `run.py` の `build.rsp` と同じ形式で、Windows Unity の `UnityReferenceAssemblies/unity-4.8-api/*.dll` を参照する。
   Windows 版 `NetCoreRuntime/dotnet.exe exec DotNetSdkRoslyn/csc.dll /noconfig @build.rsp` でコンパイルし、同梱の Mono で実行する。
   Unity 内の測定と別のファイルに保存する。インストール形態により同梱パスが異なるので、版と使用 exe の場所を記録する。

4. Windows の通常の .NET も別に採る。提供する `Probe.csproj` は .NET 8 SDK 用。Unity Mono の結果と混ぜない。

   ```powershell
   dotnet --info > target/determinism/dotnet-info.txt
   dotnet build tools/determinism/Probe.csproj -c Release
   cmd /c 'dotnet target\determinism\dotnet\bin\net8.0\Probe.dll < target\determinism\inputs.txt > target\determinism\dotnet-math.txt'
   cmd /c 'dotnet target\determinism\dotnet\bin\net8.0\Probe.dll --mathf < target\determinism\mathf-inputs.txt > target\determinism\dotnet-mathf.txt'
   ```

   `Directory.Build.props` により obj も target 内になる。.NET SDK でのビルドと Windows での実行はまだ確かめていない。

5. 入力を厳密に突き合わせて比較する。順序・行数・精度が違えばエラーになる。ULP は比較している型（32/64 ビット）で計算する。

   ```powershell
   python tools/determinism/compare.py target/determinism/unity-math.txt target/determinism/rust-msvc.txt target/determinism/windows-unity-msvc
   python tools/determinism/compare.py target/determinism/dotnet-mathf.txt target/determinism/rust-msvc-mathf.txt target/determinism/windows-dotnet-mathf
   ```

   Unity の C# Core の完全な正解再生成（Unity ブリッジのタグ `0.4.0` の `Runtime/Core` が要る。`docs/DEVELOPMENT.md`）も、元の golden を上書きせず target 内へ出す。
   入力・実行環境・出力の SHA-256 を記録し、Linux の正解と Windows Unity のどちらに合わせるか判断する。
   最適化やランタイム更新後も同じ入力で再測定する。

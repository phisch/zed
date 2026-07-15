# Vello renderer experiment

Status: implementation-complete experiment on `experiment/vello-renderer`; all work remains intentionally uncommitted for review.

## AI handoff from the repository root

Read this document before changing code.

Current Git state:

- Branch: `experiment/vello-renderer`.
- Base commit: `6b9f448ffc1d0807c57dfc94e76b1b4c4a319e7a`.
- Preserve the complete uncommitted worktree. Do not reset, clean, switch branches, or overwrite existing changes.
- The shared renderer implementation is in `crates/gpui_wgpu/src/wgpu_renderer.rs`, `crates/gpui_wgpu/src/vello_scene.rs`, and `crates/gpui_wgpu/src/vello_headless_renderer.rs`.
- The shared text system is `crates/gpui_wgpu/src/parley_text_system.rs`.
- macOS presentation is adapted in `crates/gpui_macos/src/vello_renderer.rs`.

The requested replacement is complete at this checkpoint:

- Linux Wayland/X11, web, macOS, Windows, and native headless rendering all use the shared WGPU/Vello implementation.
- GPUI's custom WGPU pipelines and WGSL shaders are deleted.
- The macOS Metal renderer, atlas, shader library, and renderer build script are deleted.
- The Windows DirectX renderer, device/atlas layers, DirectWrite text system, HLSL shaders, AMD AGS packaging, and renderer build script are deleted.
- `PlatformAtlas`, `WgpuAtlas`, atlas keys/tiles/textures, sprite scene primitives, sprite batches, and image-drop eviction APIs are deleted.
- Images and monochrome SVGs are retained scene resources consumed directly by Vello.
- Fontique discovers and resolves fonts, Parley shapes text, Skrifa supplies metrics, and Vello draws positioned glyph runs. Cosmic-text, Swash rasterization, font-kit, CoreText text shaping, DirectWrite text shaping, glyph atlases, and the obsolete text-rendering-mode setting are removed.
- Renderer-era manifest residue is removed, including `etagere`, Swash, the old macOS renderer/build dependencies from `gpui`, and unused Direct3D/DirectWrite/DXGI/HLSL Windows feature flags. None of `etagere`, Swash, font-kit, or cosmic-text remains in `Cargo.lock`.
- `cargo machete` reports no unused dependencies.

## Current architecture

1. Elements perform layout, prepaint, and paint.
2. `Window` emits renderer-independent scaled primitives into `gpui::Scene`: quads, shadows, vector paths, underlines, retained glyph runs, retained images, retained SVG trees, and macOS video surfaces.
3. Cached view paint ranges replay retained scene operations into each complete frame.
4. `vello_scene::rebuild_vello_scene` preserves cross-primitive order and translates every retained primitive into a fresh Vello scene.
5. `WgpuRenderer` renders Vello into an `Rgba8Unorm` storage texture, blits it to the configured swapchain texture, submits, and presents.
6. Resize, transparency changes, surface replacement, and device recovery invalidate and recreate size/device-dependent resources.
7. `VelloHeadlessRenderer` uses the same scene translator and resource cache without a surface and supports aligned RGBA readback.

The retained scene no longer exposes shader storage-layout details: paths retain semantic commands only, booleans are ordinary Rust `bool`s, and slash backgrounds retain semantic width/interval values rather than packed WGSL parameters. Dashed borders are encoded per enabled side and preserve per-side widths.

WGPU instance creation enables Metal, DX12, Vulkan, and GL on native targets. Vello and GPUI share one device and queue per platform GPU context.

## Text rendering and responsiveness

`ParleyTextSystem` is used by production Linux, web, macOS, and Windows platform setup. It retains the exact resolved font blob, collection index, normalized variable-font coordinates, and shaped glyph positions needed by Vello. Font identity includes the requested family, attributes, fallback stack, OpenType features, and variation coordinates, and Skrifa metrics use the same normalized coordinates as shaping/rendering.

The initial retained-text implementation emitted one Vello glyph draw and one clip layer per glyph. A full-size editor frame contained roughly 3,000 glyph primitives; startup invalidations saturated frame production and could leave a full-size X11 window black until resize reduced the scene. Combining only adjacent compatible glyphs fixed the initial startup failure, but syntax-highlighted text still alternated colors frequently enough to create hundreds or thousands of Vello clip/draw operations. This caused text-dependent lag and a grey-window threshold that decreased as more text became visible.

After that first batching pass, a real two-pane editor was still reported to render at roughly one frame per second. Renderer telemetry for a 1693×1382 two-pane syntax-highlighted frame showed approximately 15,000 glyphs, 81–104 glyph draw orders, 98–121 content-mask groups, and 480–517 final Vello glyph draws. The root cause was `Window::paint_layer` using outward-rounded raster coverage bounds for overlap ordering. Vertically adjacent editor line layers at fractional device coordinates therefore overlapped by one device pixel and received nearly one distinct draw order per line, preventing cross-line text grouping.

Layer ordering now uses exact scaled geometry through `scale_layer_bounds`; outward rounding remains limited to raster coverage and content masks. The same workload dropped to 7–8 glyph orders, 20–29 content-mask groups, and 64–84 final glyph draws. `window::tests::scaled_adjacent_layers_remain_nonoverlapping` protects this distinction.

GPUI text painting also now groups visible glyphs by font and color within each shaped line, including nonadjacent matching syntax styles, before retaining a `VectorGlyphRun`. `Window::paint_glyphs` resolves font render data and metrics once per batch and computes one union bound. This reduced a roughly 15,500-glyph multi-pane frame from about 15,000 retained glyph runs to 1,900–2,100 without changing positioned glyphs or final output. `text_system::line::tests::glyph_paint_batches_combine_nonadjacent_matching_styles` covers the source-side batching.

`vello_scene::encode_glyph_runs` partitions retained glyph primitives by draw order, then groups all runs at that order by content mask and compatible font/style, including nonadjacent syntax runs. Each content mask is applied once per draw order and each unique style becomes one Vello glyph draw. GPUI draw order guarantees that geometry sharing an order is non-overlapping, so this reordering preserves compositing while separately ordered translucent or overlapping glyphs remain isolated.

Targeted debug telemetry records primitive counts plus surface acquisition, target setup, scene rebuild, Vello encoding, blit/submit, presentation, and total renderer time. A four-pane 1693×1382 stress frame with approximately 14,600 glyphs and 2,000 retained runs produced 10 glyph orders, 28–29 content-mask groups, and 152–153 Vello draws. Steady renderer time was generally about 14–19 ms on the current RADV machine, with transient animated/indexing frames around 30 ms. The telemetry uses `log::debug!` and is inactive in normal logging configurations.

The scene retains grouped glyph runs so GPUI's cached paint ranges remain stable, while final grouping still occurs when encoding the complete frame into Vello. A stress regression renders 11,900 alternating-color one-glyph runs at 2560×1440 and verifies visible output. On the current RADV test machine the complete GPU render/readback test finishes in about 0.18 seconds.

Parley's style-split `GlyphRun` metadata now tracks consumed glyph offsets per underlying run before pairing positioned glyphs with cluster byte indices and emoji flags. Regressions verify that style segments retain the correct byte indices and that shaped glyph IDs match Skrifa's cmap for the exact font supplied to Vello.

The editor's clipped first column was a separate metric-conversion bug. GPUI's font metric contract uses a signed negative descent, and the editor negates it to obtain a positive zero-scroll gutter inset. Skrifa already returns a signed descent, but the Parley adapter negated it again, moving the editor content origin left of its text clip. Preserving Skrifa's signed descent restores the intended inset without a renderer-wide glyph offset. `parley_text_system::tests::font_metrics_use_signed_descent` and `editor_tests::test_zero_horizontal_scroll_keeps_first_column_inside_text_clip` cover both sides of this contract.

A Vello-backed regression clicks a dirty child view and verifies that a cached sibling remains present in the screenshot. This covers the production-like partial repaint/replay path in addition to the original full-root click test.

## Window lifecycle fixes

- All transient surface states request another redraw.
- Forced recovery redraws bypass GPUI's inactive-window frame throttle, so Linux cannot consume and discard a renderer retry.
- Foreground-thread waits were removed from resize and device recovery.
- Uncaptured WGPU errors are logged immediately and retained for the next draw/recovery decision.
- Transparent surfaces prefer premultiplied or postmultiplied alpha and never silently choose an opaque mode for a window initially requested as transparent.
- Vello device creation uses `wgpu::Limits::default()`.

## macOS video surfaces

macOS `CVPixelBuffer` surfaces remain functional through Vello. NV12 full-range buffers are converted into cached retained RGBA `ImageData`; the source buffer is retained and the cache uses pointer identity only while the source remains active, preventing address-reuse bugs. A stride-aware unit test covers conversion from independently padded luma and chroma planes.

This is a correctness-first CPU conversion. The next useful optimization is a GPU-native `CVPixelBuffer`/IOSurface import and YUV conversion/compositing path. It is not a functional blocker, but high-resolution video can make the current CPU path expensive.

## Retained images and SVGs

- Decoded images are retained by `Arc<RenderImage>` and converted to Peniko image data by image/frame/grayscale identity. Weak cache entries are pruned when the source is dropped; no explicit atlas eviction remains.
- GPUI's `Svg` element remains monochrome by contract. Parsed `usvg::Tree` resources are retained and cached; Vello applies the GPUI foreground tint by isolating the foreground and compositing SVG alpha with Porter-Duff `DestIn`.
- Full-color SVG-as-image consumers still use the existing CPU SVG raster path. That path is content functionality rather than a renderer backend.
- Published `vello_svg 0.9.0` depends on Vello 0.7. Keep revision `2db0427e4dc9879bd636caf69e60dee026d83699`, which is compatible with Vello 0.9, until a compatible release is published.
- `vello_svg` still has upstream limitations around some patterns, masks, filters, and complex clip paths; continue parity testing when those assets are encountered.

## Headless rendering and regressions

The native headless renderer owns a surface-independent WGPU context, Vello renderer/scene, retained resource cache, and reusable `Rgba8Unorm` storage texture with `COPY_SRC`.

Coverage includes:

- primitive ordering and clipping;
- gradients, shadows, paths, per-side dashed borders, and underlines;
- retained image conversion/cache identity and rounded clipping;
- retained monochrome SVG tinting and cache identity;
- Parley font registration, fallback, shaping, signed metrics, style-segment metadata, cmap identity, and exact Vello font-resource sharing;
- visible Parley/Vello text through `HeadlessAppContext`;
- 11,900 alternating-color glyph primitives rendered at 2560×1440;
- nonadjacent source glyph batching and same-order Vello style grouping with draw-order boundaries preserved;
- exact adjacent layer overlap ordering at fractional scale factors;
- a positive editor first-column inset at zero horizontal scroll;
- mouse down/up dispatch changing rendered pixels;
- partial repaint replay preserving a cached sibling;
- resize target recreation and newly exposed pixels;
- row-aligned RGBA readback;
- stride-aware macOS NV12 conversion;
- `gpui_platform::current_headless_renderer` producing RGBA pixels.

## Runtime verification

A debug `zed` build was exercised in a real niri session on RADV/Vulkan.

X11/XWayland, isolated profile:

- initial full-size 1693×1382 restricted-worktree modal rendered;
- Return dismissed the modal;
- `Ctrl+Shift+P`, text entry, and Escape updated immediately;
- a real mouse click moved focus into the editor;
- resize to 700×500 and then 1500×1100 preserved complete content;
- the command palette reopened correctly after resizing;
- no WGPU validation, device-loss, surface-loss, or Vello errors were logged.

Wayland-native, isolated profile:

- initial full-size 1693×1382 modal and editor rendered;
- niri resized the live window to 700×500 and then 1500×1100;
- modal, editor, text, and newly exposed regions remained rendered at every size;
- no WGPU validation, device-loss, surface-loss, or Vello errors were logged.

A final post-audit rerun verified the same full-size startup after the draw-order-aware batching change. Wayland-native resize captures remained complete at 700×500 and 1500×1100, and an X11 run dismissed the restricted-worktree modal, opened the command palette, accepted `theme selector` input, and rendered matching results immediately. Captures are `target/zed-vello-post-audit-2-startup.png`, `target/zed-vello-post-audit-2-small.png`, `target/zed-vello-post-audit-2-large.png`, and `target/zed-vello-post-audit-3-palette.png`.

After the residual syntax-highlighting bottleneck was reported and fixed, a debug X11 editor displaying `vello_scene.rs` remained fully rendered while resizing from 1693×1382 through 3000×1300 to 3400×1382. The previous text-dependent grey threshold did not reproduce. The full release binary was then built and independently verified with the same syntax-highlighted editor at 3400×1382. Captures are `target/zed-vello-text-batched-1693.png`, `target/zed-vello-text-batched-3000.png`, `target/zed-vello-text-batched-3400.png`, and `target/zed-vello-text-release-3400.png`. The bounded-run stderr logs contained no WGPU, Vello, surface, device-loss, or panic messages.

Following the exact layer-order, source batching, and signed-descent fixes, the latest debug binary restored four syntax-heavy editor panes at 1693×1382. Repeated PageDown input updated immediately, a real mouse click focused the editor, the command palette accepted `theme selector` without delay, and resizing through 700×500 and 2500×1300 kept every exposed region rendered. The first character in each editor pane is fully visible after the gutter. Captures are `target/vello-live-start.png`, `target/vello-live-scrolled.png`, `target/vello-live-palette-focused.png`, `target/vello-live-small.png`, and `target/vello-live-large.png`.

The final optimized release binary independently passed the same four-pane interaction check, command-palette input, and a 2500×1300 resize without grey or stale regions. Captures are `target/vello-release-live-start.png`, `target/vello-release-live-palette.png`, and `target/vello-release-live-large.png`.

The shared application log also contains messages from the separately running installed Preview build; its legacy Swash terminal errors do not originate from this experiment binary. The experiment source and lockfile contain no Swash dependency or rendering path.

The isolated profiles logged expected authentication failures for unavailable account services; these are unrelated to rendering.

## Passing validation

```sh
cargo fmt --all -- --check
cargo test -p gpui --lib
cargo test -p editor --lib
cargo test -p gpui_wgpu --lib
cargo test -p gpui_wgpu --no-default-features --features test-support --lib
cargo test -p gpui_platform --features test-support \
  headless_renderer_tests::current_headless_renderer_produces_rgba_pixels
cargo check -p gpui_wgpu --target wasm32-unknown-unknown
cargo check -p gpui --target wasm32-unknown-unknown
cargo check -p zed
cargo build -p zed
cargo build -p zed --release
./script/clippy -p gpui_wgpu -p gpui_platform -p gpui_macos -p gpui_windows --lib
cargo machete
git diff --check
```

Cross-host platform checks also pass:

```sh
env CC_x86_64_apple_darwin=clang \
  CFLAGS_x86_64_apple_darwin=--target=x86_64-apple-darwin \
  cargo check -p gpui_macos --features test-support \
  --target x86_64-apple-darwin

env CC_x86_64_pc_windows_msvc=clang-cl \
  CFLAGS_x86_64_pc_windows_msvc=/I/home/phisch/code/phisch/zed/target/fake-windows-sdk \
  cargo check -p gpui_windows --no-default-features \
  --features test-support --target x86_64-pc-windows-msvc
```

`target/fake-windows-sdk/windows.h` is an ignored cross-host check shim containing only the declaration needed by `stacker`; it is not part of the experiment and must not be committed.

## Important constraints

- Keep Vello device limits at `wgpu::Limits::default()`; downlevel limits reproduce black output.
- Do not add `RENDER_ATTACHMENT` use or a render pass targeting Vello's storage texture.
- Keep immediate logging and consumption of uncaptured WGPU errors.
- Preserve forced redraw behavior for transient surface failures and recovery.
- Preserve exact layer overlap geometry and both source/final glyph-run batching; outward-rounded layer ordering or one retained/encoded draw per glyph makes a multi-pane editor effectively unresponsive.
- Keep the pinned `vello_svg` revision until the published crate is Vello-0.9-compatible.

## Recommended next bounded task

The renderer replacement has no known functional blocker after the multi-pane performance and editor inset fixes. The next bounded renderer task is to optimize macOS video by importing `CVPixelBuffer`/IOSurface planes into WGPU and performing YUV conversion on the GPU while retaining the current CPU path as a tested fallback. Validate video playback, source-buffer lifetime, device recovery, and both full- and video-range color conversion on real Apple hardware.

## Sources

- Vello: <https://github.com/linebender/vello>
- Vello 0.9 renderer API: <https://docs.rs/vello/0.9.0/vello/struct.Renderer.html>
- Vello SVG: <https://github.com/linebender/vello_svg>
- Vello-0.9 SVG compatibility revision: <https://github.com/linebender/vello_svg/commit/2db0427e4dc9879bd636caf69e60dee026d83699>
- Parley: <https://docs.rs/parley/0.11.0/parley/>
- Fontique: <https://docs.rs/fontique/>

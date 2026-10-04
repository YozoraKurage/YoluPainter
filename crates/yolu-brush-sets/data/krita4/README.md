# Krita 4 default brush tips

The brush tip images in `brushes/` are taken unchanged from Krita's
`Krita_4_Default_Resources.bundle`:

- Source: https://invent.kde.org/graphics/krita/-/blob/master/krita/data/bundles/Krita_4_Default_Resources.bundle
  (bundle SHA-256 `4180f474052305e9de6eaac1d832c1ddeb0654142bcde2eb2460629cf23796d0`, retrieved 2026-10-02)
- License: **CC0 1.0** (public domain dedication), as declared in the bundle's `meta.xml`
  (`<meta:meta-userdefined meta:name="license" meta:value="CC-0"/>`). The original `meta.xml` is kept here verbatim.
- Credit: David Revoy (Deevad), with derivations of the brushes of Ramon Miranda, Razvanc, Radian, Wolthera, Storm,
  Scottyp and others, for the Krita project (https://krita.org). CC0 does not require attribution; it is given out of
  courtesy and because a third-party mirror of this bundle has been seen labelled CC BY 3.0.

What is included: the 76 raster tips (`.png`, `.gih`, `.gbr`). The 3 SVG tips are left out (this tool does not render
SVG), and so are the bundle's Krita paint-op presets (`.kpp`, Krita brush-engine settings) and patterns. YoluPainter
gives each tip its own default settings; they are not Krita's presets.

`SHA256SUMS` lists every included file so the copy can be checked against the bundle.

How the tips are read: `.gbr` / `.gih` follow the GIMP formats (255 = full paint). `.png` tips follow Krita's convention:
dark paints and white or transparent does not, so coverage = (1 − luminance) × alpha.

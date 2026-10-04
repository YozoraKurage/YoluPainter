//! 上限付きのカラーセット入出力。形式と互換性は README.md を参照。
use super::{Error, Palette, Swatch, MAX_COLORS, MAX_FILE_BYTES, MAX_NAME_BYTES};
use crate::state::{hsv_to_rgb, Rgba};

pub fn valid_name(name: &str) -> bool {
    name.len() <= MAX_NAME_BYTES && !name.chars().any(char::is_control)
}
pub fn valid_color(color: Rgba) -> bool {
    color
        .iter()
        .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
}
pub fn validate(p: &Palette) -> Result<(), Error> {
    if p.colors.len() > MAX_COLORS {
        return Err(Error::Limit);
    }
    if !valid_name(&p.name) || p.colors.iter().any(|c| !valid_name(&c.name)) {
        return Err(Error::Name);
    }
    if p.colors.iter().any(|c| !valid_color(c.rgba)) {
        return Err(Error::Color);
    }
    Ok(())
}
fn bounded(bytes: &[u8]) -> Result<(), Error> {
    if bytes.len() > MAX_FILE_BYTES {
        Err(Error::Limit)
    } else {
        Ok(())
    }
}

/// GPL v1/v2。格子の列数は読み取り時に検証し、表示幅に応じて再配置する。
pub fn read_gpl(bytes: &[u8], fallback: &str) -> Result<Palette, Error> {
    bounded(bytes)?;
    let text = std::str::from_utf8(bytes).map_err(|_| Error::Encoding)?;
    let mut lines = text.lines();
    if lines.next() != Some("GIMP Palette") {
        return Err(Error::Header);
    }
    let mut p = Palette {
        name: fallback.into(),
        colors: Vec::new(),
    };
    let mut lines = lines.peekable();
    if let Some(name) = lines.peek().and_then(|l| l.strip_prefix("Name:")) {
        p.name = name.trim().into();
        lines.next();
        if let Some(cols) = lines.peek().and_then(|l| l.strip_prefix("Columns:")) {
            cols.trim().parse::<u8>().map_err(|_| Error::Header)?;
            lines.next();
        }
    }
    for line in lines {
        let mut rest = line.trim();
        if rest.is_empty() || rest.starts_with('#') {
            continue;
        }
        if p.colors.len() == MAX_COLORS {
            return Err(Error::Limit);
        }
        let mut rgba = [0.0, 0.0, 0.0, 1.0];
        for channel in &mut rgba[..3] {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            *channel = rest[..end].parse::<u8>().map_err(|_| Error::Color)? as f32 / 255.0;
            rest = rest[end..].trim_start();
        }
        p.colors.push(Swatch {
            name: rest.trim().into(),
            rgba,
        });
    }
    validate(&p)?;
    Ok(p)
}
pub fn write_gpl(p: &Palette) -> Result<Vec<u8>, Error> {
    validate(p)?;
    if p.colors.iter().any(|c| c.rgba[3] != 1.0) {
        return Err(Error::Alpha);
    }
    let mut s = format!("GIMP Palette\nName: {}\nColumns: 0\n#\n", p.name);
    for c in &p.colors {
        let [r, g, b, _] = c.rgba.map(crate::ui::widgets::to_byte);
        s += &format!("{r} {g} {b}\t{}\n", c.name);
    }
    bounded(s.as_bytes())?;
    Ok(s.into_bytes())
}

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        if n > self.0.len() {
            return Err(Error::Truncated);
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }
    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn text(&mut self) -> Result<String, Error> {
        let len = self.u16()? as usize;
        if len > MAX_NAME_BYTES {
            return Err(Error::Name);
        }
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| Error::Encoding)
    }
}
fn aco_section(r: &mut Reader<'_>, version: u16) -> Result<Vec<Swatch>, Error> {
    if ![1, 2].contains(&version) {
        return Err(Error::Version);
    }
    let n = r.u16()? as usize;
    if n > MAX_COLORS {
        return Err(Error::Limit);
    }
    let mut colors = Vec::with_capacity(n);
    for _ in 0..n {
        let space = r.u16()?;
        let v = [r.u16()?, r.u16()?, r.u16()?, r.u16()?];
        let f = v.map(|n| n as f32 / 65535.0);
        let rgba = match space {
            0 => [f[0], f[1], f[2], 1.0],
            1 => {
                let (red, green, blue) = hsv_to_rgb(f[0], f[1], f[2]);
                [red, green, blue, 1.0]
            }
            8 if v[0] <= 10000 => {
                let gray = v[0] as f32 / 10000.0;
                [gray, gray, gray, 1.0]
            }
            8 => return Err(Error::Color),
            _ => return Err(Error::ColorSpace(space)),
        };
        let name = if version == 2 {
            let len = r.u32()? as usize;
            if len == 0 {
                return Err(Error::Encoding);
            }
            if len > MAX_NAME_BYTES + 1 {
                return Err(Error::Name);
            }
            let mut utf16 = Vec::with_capacity(len);
            for _ in 0..len {
                utf16.push(r.u16()?);
            }
            if utf16.pop() != Some(0) {
                return Err(Error::Encoding);
            }
            String::from_utf16(&utf16).map_err(|_| Error::Encoding)?
        } else {
            String::new()
        };
        colors.push(Swatch { name, rgba });
    }
    Ok(colors)
}
pub fn read_aco(bytes: &[u8], name: &str) -> Result<Palette, Error> {
    bounded(bytes)?;
    let mut r = Reader(bytes);
    let version = r.u16()?;
    let mut colors = aco_section(&mut r, version)?;
    if version == 1 && !r.0.is_empty() {
        if r.u16()? != 2 {
            return Err(Error::Version);
        }
        let named = aco_section(&mut r, 2)?;
        if named.len() != colors.len() || named.iter().zip(&colors).any(|(a, b)| a.rgba != b.rgba) {
            return Err(Error::Mismatch);
        }
        colors = named;
    }
    if !r.0.is_empty() {
        return Err(Error::Trailing);
    }
    let p = Palette {
        name: name.into(),
        colors,
    };
    validate(&p)?;
    Ok(p)
}

pub fn encode(p: &Palette) -> Result<Vec<u8>, Error> {
    validate(p)?;
    let mut out = b"YCS\0\0\x01".to_vec();
    fn text(out: &mut Vec<u8>, s: &str) {
        out.extend((s.len() as u16).to_be_bytes());
        out.extend(s.as_bytes());
    }
    text(&mut out, &p.name);
    out.extend((p.colors.len() as u16).to_be_bytes());
    for c in &p.colors {
        text(&mut out, &c.name);
        for v in c.rgba {
            out.extend(v.to_be_bytes());
        }
    }
    bounded(&out)?;
    Ok(out)
}
pub fn decode(bytes: &[u8]) -> Result<Palette, Error> {
    bounded(bytes)?;
    let mut r = Reader(bytes);
    if r.take(4)? != b"YCS\0" {
        return Err(Error::Header);
    }
    if r.u16()? != 1 {
        return Err(Error::Version);
    }
    let name = r.text()?;
    let n = r.u16()? as usize;
    if n > MAX_COLORS {
        return Err(Error::Limit);
    }
    let mut colors = Vec::with_capacity(n);
    for _ in 0..n {
        let name = r.text()?;
        let mut rgba = [0.0; 4];
        for v in &mut rgba {
            *v = f32::from_be_bytes(r.take(4)?.try_into().unwrap());
        }
        colors.push(Swatch { name, rgba });
    }
    if !r.0.is_empty() {
        return Err(Error::Trailing);
    }
    let p = Palette { name, colors };
    validate(&p)?;
    Ok(p)
}

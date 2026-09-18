//! Formatação de unidades (binárias, explícitas: KiB, MiB, GiB) e tempo,
//! com vírgula decimal (pt-BR).

pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut v = n as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    let s = if v >= 100.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    };
    format!("{} {}", s.replace('.', ","), UNITS[unit])
}

pub fn speed(bytes_per_sec: u64) -> String {
    format!("{}/s", bytes(bytes_per_sec))
}

pub fn duration(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h} h {m} min")
    } else if m > 0 {
        format!("{m} min {s} s")
    } else {
        format!("{s} s")
    }
}

/// Tempo restante estimado; `None` quando tamanho ou velocidade são
/// desconhecidos (nunca inventa uma estimativa).
pub fn eta(total: Option<u64>, done: u64, speed: u64) -> Option<u64> {
    let total = total?;
    if speed == 0 || done > total {
        return None;
    }
    Some((total - done).div_ceil(speed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_units_with_comma() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(1536), "1,5 KiB");
        assert_eq!(bytes(5 * 1024 * 1024), "5,0 MiB");
        assert_eq!(bytes(3 * 1024 * 1024 * 1024 + 512 * 1024 * 1024), "3,5 GiB");
        assert_eq!(bytes(200 * 1024 * 1024), "200 MiB");
        assert_eq!(speed(2 * 1024 * 1024), "2,0 MiB/s");
    }

    #[test]
    fn eta_unknown_when_no_size_or_speed() {
        assert_eq!(eta(None, 10, 100), None);
        assert_eq!(eta(Some(1000), 10, 0), None);
        assert_eq!(eta(Some(1000), 0, 100), Some(10));
    }

    #[test]
    fn durations() {
        assert_eq!(duration(42), "42 s");
        assert_eq!(duration(200), "3 min 20 s");
        assert_eq!(duration(3900), "1 h 5 min");
    }
}

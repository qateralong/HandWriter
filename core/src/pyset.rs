const XXPRIME_1: u64 = 11400714785074694791;
const XXPRIME_2: u64 = 14029467366897019727;
const XXPRIME_5: u64 = 2870177450012600261;

pub fn hash_int(v: i64) -> u64 {
    if v == -1 { (-2i64) as u64 } else { v as u64 }
}

pub fn hash_pair(a: i64, b: i64) -> u64 {
    let mut acc = XXPRIME_5;
    for lane in [hash_int(a), hash_int(b)] {
        acc = acc.wrapping_add(lane.wrapping_mul(XXPRIME_2));
        acc = acc.rotate_left(31);
        acc = acc.wrapping_mul(XXPRIME_1);
    }
    acc = acc.wrapping_add(2 ^ (XXPRIME_5 ^ 3527539));
    if acc == u64::MAX { 1546275796 } else { acc }
}

#[derive(Clone, Copy)]
enum Slot<K> {
    Empty,
    Dummy,
    Used(K, u64),
}

pub struct PySet<K> {
    table: Vec<Slot<K>>,
    mask: usize,
    fill: usize,
    used: usize,
}

const LINEAR_PROBES: usize = 9;
const PERTURB_SHIFT: u32 = 5;

impl<K: Copy + PartialEq> PySet<K> {
    pub fn new() -> Self {
        Self { table: vec![Slot::Empty; 8], mask: 7, fill: 0, used: 0 }
    }

    pub fn is_empty(&self) -> bool {
        self.used == 0
    }

    pub fn add(&mut self, key: K, hash: u64) {
        let mask = self.mask;
        let mut i = (hash as usize) & mask;
        let mut perturb = hash;
        let mut freeslot: Option<usize> = None;
        loop {
            let mut probes = if i + LINEAR_PROBES <= mask { LINEAR_PROBES } else { 0 };
            let mut k = i;
            loop {
                match self.table[k] {
                    Slot::Empty => {
                        let slot = freeslot.unwrap_or(k);
                        if freeslot.is_some() {
                            self.table[slot] = Slot::Used(key, hash);
                            self.used += 1;
                            return;
                        }
                        self.table[slot] = Slot::Used(key, hash);
                        self.fill += 1;
                        self.used += 1;
                        if self.fill * 5 >= mask * 3 {
                            self.resize(if self.used > 50000 { self.used * 2 } else { self.used * 4 });
                        }
                        return;
                    }
                    Slot::Used(k2, h2) if h2 == hash && k2 == key => return,
                    Slot::Dummy if freeslot.is_none() => freeslot = Some(k),
                    _ => {}
                }
                if probes == 0 {
                    break;
                }
                probes -= 1;
                k += 1;
            }
            perturb >>= PERTURB_SHIFT;
            i = (i.wrapping_mul(5).wrapping_add(1).wrapping_add(perturb as usize)) & mask;
        }
    }

    fn insert_clean(table: &mut [Slot<K>], mask: usize, key: K, hash: u64) {
        let mut perturb = hash;
        let mut i = (hash as usize) & mask;
        loop {
            if matches!(table[i], Slot::Empty) {
                table[i] = Slot::Used(key, hash);
                return;
            }
            if i + LINEAR_PROBES <= mask {
                for j in 1..=LINEAR_PROBES {
                    if matches!(table[i + j], Slot::Empty) {
                        table[i + j] = Slot::Used(key, hash);
                        return;
                    }
                }
            }
            perturb >>= PERTURB_SHIFT;
            i = (i.wrapping_mul(5).wrapping_add(1).wrapping_add(perturb as usize)) & mask;
        }
    }

    fn resize(&mut self, minused: usize) {
        let mut newsize = 8;
        while newsize <= minused {
            newsize <<= 1;
        }
        let old = std::mem::replace(&mut self.table, vec![Slot::Empty; newsize]);
        self.mask = newsize - 1;
        for s in old {
            if let Slot::Used(k, h) = s {
                Self::insert_clean(&mut self.table, self.mask, k, h);
            }
        }
        self.fill = self.used;
    }

    pub fn discard(&mut self, key: K, hash: u64) {
        let mask = self.mask;
        let mut i = (hash as usize) & mask;
        let mut perturb = hash;
        loop {
            let mut probes = if i + LINEAR_PROBES <= mask { LINEAR_PROBES } else { 0 };
            let mut k = i;
            loop {
                match self.table[k] {
                    Slot::Empty => return,
                    Slot::Used(k2, h2) if h2 == hash && k2 == key => {
                        self.table[k] = Slot::Dummy;
                        self.used -= 1;
                        return;
                    }
                    _ => {}
                }
                if probes == 0 {
                    break;
                }
                probes -= 1;
                k += 1;
            }
            perturb >>= PERTURB_SHIFT;
            i = (i.wrapping_mul(5).wrapping_add(1).wrapping_add(perturb as usize)) & mask;
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = K> + '_ {
        self.table.iter().filter_map(|s| match s {
            Slot::Used(k, _) => Some(*k),
            _ => None,
        })
    }
}

impl<K: Copy + PartialEq> Default for PySet<K> {
    fn default() -> Self {
        Self::new()
    }
}

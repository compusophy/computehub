//! Fuel: a draining execution budget. An evaluator that burns fuel on every
//! step is total by construction ("halts within N units"). Every
//! sub-evaluation spends from the same `&mut Fuel`, never a fresh tank, so
//! composed programs halt too. Zero dependencies; native and wasm32.
//!
//! Forked from litelite's fuellite 0.2.0 (commit 4f5e056).

#![forbid(unsafe_code)]

/// The budget is spent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exhausted;

/// A draining execution budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fuel {
    remaining: u64,
}

impl Fuel {
    pub fn new(units: u64) -> Self {
        Self { remaining: units }
    }

    /// Spends `cost` units; when fewer remain, empties the tank and fails.
    pub fn burn(&mut self, cost: u64) -> Result<(), Exhausted> {
        if cost > self.remaining {
            self.remaining = 0;
            return Err(Exhausted);
        }
        self.remaining -= cost;
        Ok(())
    }

    pub fn remaining(&self) -> u64 {
        self.remaining
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuel_burns_down_and_over_burn_empties_the_tank() {
        let mut f = Fuel::new(10);
        assert_eq!((f.burn(4), f.burn(6), f.remaining()), (Ok(()), Ok(()), 0));
        assert_eq!(f.burn(1), Err(Exhausted));
        let mut f = Fuel::new(5);
        assert_eq!((f.burn(9), f.remaining()), (Err(Exhausted), 0));
    }

    #[test]
    fn one_tank_bounds_a_whole_composition() {
        // Parent and recursive children share one tank: a program that never
        // returns on its own still halts, by exhaustion.
        fn spin(f: &mut Fuel, depth: usize) -> Result<(), Exhausted> {
            loop {
                f.burn(1)?;
                if depth > 0 {
                    spin(f, depth - 1)?;
                }
            }
        }
        let mut f = Fuel::new(10_000);
        assert_eq!((spin(&mut f, 5), f.remaining()), (Err(Exhausted), 0));
        let mut f = Fuel::new(100);
        (0..40).for_each(|_| f.burn(2).unwrap());
        assert_eq!(f.remaining(), 20);
    }
}

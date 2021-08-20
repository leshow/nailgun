use std::fmt;

use num_traits::{AsPrimitive, Num};

pub struct MovingAvg<T> {
    val: T,
    interval: usize,
}

impl<T> MovingAvg<T> {
    pub fn new(val: T) -> Self {
        Self { val, interval: 0 }
    }
}

impl<T> MovingAvg<T>
where
    T: Num + AsPrimitive<T>,
    usize: AsPrimitive<T>,
{
    pub fn next(&mut self, val: T) -> T {
        self.interval += 1;
        self.val = (self.val * (self.interval - 1).as_() + val) / (self.interval).as_();
        self.val
    }
}

impl<T: fmt::Display> fmt::Display for MovingAvg<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.val)
    }
}

impl<T: fmt::Debug> fmt::Debug for MovingAvg<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.val)
    }
}

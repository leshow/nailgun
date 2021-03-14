use std::time::Instant;

pub(crate) struct TokenBucket {
    qps: f64,
    wallet: u64,
    last_fill: Option<Instant>,
}

impl TokenBucket {
    pub(crate) fn new(qps: f64) -> Self {
        Self {
            qps,
            wallet: 0,
            last_fill: None,
        }
    }

    pub(crate) fn consume(&mut self, tokens: u64, now: Instant) -> bool {
        if self.wallet < tokens {
            match self.last_fill {
                None => {
                    self.last_fill = Some(Instant::now());
                }
                Some(last) if now > last => {
                    let elapsed_ms = now.duration_since(last).as_millis() as f64;
                    let add = (self.qps * elapsed_ms / 1000.) as u64;
                    if self.wallet + add >= tokens {
                        self.wallet += add;
                        self.last_fill = Some(now);
                    }
                    if self.wallet < tokens {
                        return false;
                    }
                }
                _ => {}
            }
        }
        self.wallet -= tokens;
        true
    }
}

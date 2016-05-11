// Copyright 2016 PingCAP, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// See the License for the specific language governing permissions and
// limitations under the License.

use std::boxed::{Box, FnBox};
use std::io::ErrorKind::WouldBlock;
use std::io::{Read, Write};
use std::convert::AsMut;

use kvproto::msgpb;

use super::{Result, Error};

pub type OnResponseResult = Result<Option<msgpb::Message>>;
pub type OnResponse = Box<FnBox(OnResponseResult) + Send>;

pub struct Body {
    pub pos: usize,
    pub data: Vec<u8>,
}

impl Body {
    pub fn read_from<T: Read>(&mut self, r: &mut T) -> Result<()> {
        debug!("try to read body, read pos: {}, total {}",
               self.pos,
               self.data.len());

        if self.pos >= self.data.len() {
            return Ok(());
        }

        match r.read(&mut self.data[self.pos..]) {
            Ok(0) => Err(box_err!("remote has closed the connection")),
            Ok(n) => {
                self.pos += n;
                Ok(())
            }
            Err(e) => {
                if e.kind() == WouldBlock {
                    Ok(())
                } else {
                    Err(Error::Io(e))
                }
            }
        }
    }

    pub fn write_to<T: Write>(&mut self, w: &mut T) -> Result<()> {
        debug!("try to write body, write pos: {}, total {}",
               self.pos,
               self.data.len());

        if self.pos >= self.data.len() {
            return Ok(());
        }

        match w.write(&self.data[self.pos..]) {
            Ok(0) => Err(box_err!("can't write ZERO data")),
            Ok(n) => {
                self.pos += n;
                Ok(())
            }
            Err(e) => {
                if e.kind() == WouldBlock {
                    Ok(())
                } else {
                    Err(Error::Io(e))
                }
            }
        }
    }

    pub fn remaining(&self) -> usize {
        if self.pos >= self.data.len() {
            return 0;
        }

        self.data.len() - self.pos
    }

    pub fn reset(&mut self, size: usize) {
        self.pos = 0;
        self.data.resize(size, 0);
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.len() == 0
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }
}

impl Default for Body {
    fn default() -> Body {
        Body {
            pos: 0,
            data: vec![],
        }
    }
}

impl AsMut<Vec<u8>> for Body {
    fn as_mut(&mut self) -> &mut Vec<u8> {
        &mut self.data
    }
}


#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;
    use super::super::http_client::*;
    use super::super::http_server::*;

    use mio::tcp::TcpListener;

    use kvproto::msgpb::{Message, MessageType};

    struct TestServerHandler;

    impl ServerHandler for TestServerHandler {
        fn on_request(&mut self, msg: Message, cb: OnResponse) {
            cb.call_box((Ok(Some(msg)),))
        }
    }

    #[test]
    fn test_http() {
        let addr = "127.0.0.1:0".parse().unwrap();
        let listener = TcpListener::bind(&addr).unwrap();

        let addr = listener.local_addr().unwrap();
        let url: Url = format!("http://{}{}", addr, V1_MSG_PATH).parse().unwrap();

        let mut s = Server::new(TestServerHandler);
        let listening = s.run(listener).unwrap();

        let mut msg = Message::new();
        msg.set_msg_type(MessageType::Raft);

        let c = Client::new().unwrap();
        for _ in 0..2 {
            let (tx, rx) = mpsc::channel();
            c.post_message(url.clone(),
                           msg.clone(),
                           box move |res| {
                               tx.send(res).unwrap();
                           })
             .unwrap();

            let msg1 = rx.recv().unwrap().unwrap().unwrap();
            assert!(msg1.get_msg_type() == MessageType::Raft);
        }


        let msg1 = c.post_message_timeout(url.clone(), msg.clone(), Duration::from_secs(1))
                    .unwrap()
                    .unwrap();
        assert!(msg1.get_msg_type() == MessageType::Raft);

        // TODO: add more tests.

        c.close();

        listening.close();
    }
}

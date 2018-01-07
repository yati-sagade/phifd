# phifd - Rust implementation of the Phi accrual failure detector

## Concepts

The [Phi accrual failure detector][1] enables applications to respond to
group members' suspicion levels, as opposed to binary {failed,alive} decisions
made by the failure detector itself.

For each member in the group, the FD outputs a suspicion value, guaranteed to
increase monotonically, and to tend to infinity for failed nodes. Applications
can contol the tradeoff between fast detection and high accuracy by tweaking
the threshold suspicion level `phi` at which a node is deemed failed.

A low value of `phi` will result in fast failure detections, at the expense of
increase false alarms, whereas increasing `phi` will make detections more
accurate, but will cause true detections of true failures to be delayed
accordingly.

One can hence build a group membership service on top of this FD by thresholding
appropriately the suspicion values for each node.


## Code

### High level

This crate is based on the [tokio][2] platform. The entry point is
`PhiFD::run()` in `src/lib.rs`. The execution is modeled as two main [future
streams][3] flowing into a future sink. The first stream is a stream of periodic
pings sent out to peers. The second stream is the stream of incoming pings from
peers, in response to each of which we must update the failure detector state.
The fist stream (call it the pinger) doesn't actually send anything, but
produces, as events, a `Vec` of pings that need to be sent out. The second
stream (called the ping listener) wraps the stream of incoming pings such that
upon each ping, the internal state is update as a side effect, and
a confirmation of this is emitted as an event.

These two streams are then combined into one by allowing them to "race" each
other -- this new stream emits values as values become available on either the
pinger or the ping listener streams. So each value in this new stream is either
an acknowledgement of an incoming ping being successfully processed (on the ping
listener stream), or a `Vec` of pings to be sent out. So we derive yet another
stream from this by filtering only for values of the latter kind (outstanding
pings). This stream is then ["forwarded"][4] to a sink wrapping over a UDP
socket. This completes the core flow of the code.

It is important to note that with most async frameworks, and with tokio in
particular, specifying what your stream does is the bulk of the work, but we
need to reduce all of this down to a future that can then be executed to
completion by the event loop. As such, the [forward][4] method on streams
returns a future representing the work of pumping the invocant stream into the
sink in question. This future is then run on the event loop, and once it starts
running, all the participant streams and sinks crank into action.


### Implementation details


#### State management

All state is kept in the `FDState` struct, which is held by the `PhiFD` struct
behind a `Rc<RefCell<.>>`. Read [this chapter][5] if you're not familiar with
those wrapper types. In short, an _immutable_ `RefCell<FDState>` can be used to
mutate the underlying `FDState` value, with R/W borrow checks happening at
runtime. The `Rc` smart pointer is a refcounted container that allows multiple
owners of a value (which normally is forbidden in Rust). This is necessary when
we need to access state mutably from multiple closures that arise as callbacks
in futures based code.

#### Transport

The pinging happens over UDP with each datagram containing a protobuf
serialized gossip message. The definition can be found in `proto/msg.proto`.
The protobuf compiler is invoked upon every build in `build.rs`, and this
generates the Rust structs in `src/proto`.

To decode and encode ping messages transparently, and to make available the
stream abstraction for incoming messages, we have a "codec" (tokio terminology)
called `GossipCodec` in `src/lib.rs`. It basically describes what to do with
incoming messages and how to serialize outgoing messages (and to whom should
they be sent). In our case, we just use methods on our `rust-protobuf` generated
structs. A minor detail here is that the codec works with tuples of
`(SocketAddr, Gossip)`. In other words, it decodes incoming datagrams into
a tuple of the sender address and the decoded ping message contents, and encodes
_tuples_ of (_recipient address_, ping message) into datagrams containing the
serialized message addressed to the given recipient. Once the codec is in place,
tokio's ["framed"][6] abstraction gives a stream and a sink representing
incoming messages, and a hole for outgoing messages, for the underlying UDP
socket, respectively.

[1]: http://fubica.lsd.ufcg.edu.br/hp/cursos/cfsc/papers/hayashibara04theaccrual.pdf
[2]: https://tokio.rs/
[3]: https://tokio.rs/docs/getting-started/streams-and-sinks/
[4]: https://docs.rs/futures/0.1/futures/stream/trait.Stream.html#method.forward
[5]: https://doc.rust-lang.org/nightly/book/second-edition/ch15-00-smart-pointers.html
[6]: https://docs.rs/tokio-core/0.1/tokio_core/net/struct.UdpFramed.html


#!/usr/bin/env python3
"""Drop the second LDAP simple bind for one service DN.

Koreograf binds as the service account, checks the user's password, then
binds as the service account again to read groups. A wrong password would
fail the first bind and refuse sign-in. This proxy forwards the first
service bind and closes the connection when the second one arrives.

cargo test does not run this file. The lab script starts it.
"""

from __future__ import annotations

import argparse
import select
import socket
import sys


class ProxyError(Exception):
    pass


def encode_len(length: int) -> bytes:
    if length < 0x80:
        return bytes([length])
    raw = length.to_bytes((length.bit_length() + 7) // 8, "big")
    return bytes([0x80 | len(raw)]) + raw


def tlv(tag: int, value: bytes) -> bytes:
    return bytes([tag]) + encode_len(len(value)) + value


def read_len(buf: bytes, index: int) -> tuple[int, int] | None:
    if index >= len(buf):
        return None
    first = buf[index]
    index += 1
    if first < 0x80:
        return first, index
    count = first & 0x7F
    # Indefinite length, or a length that does not fit a lab message.
    if count == 0 or count > 4:
        raise ProxyError("unsupported BER length")
    if index + count > len(buf):
        return None
    return int.from_bytes(buf[index : index + count], "big"), index + count


def message_end(buf: bytes) -> int | None:
    if not buf:
        return None
    if buf[0] != 0x30:
        raise ProxyError("not an LDAP message")
    parsed = read_len(buf, 1)
    if parsed is None:
        return None
    length, start = parsed
    end = start + length
    if end > len(buf):
        return None
    return end


def bind_dn(message: bytes) -> bytes | None:
    """DN from a simple bind, or None when the message is not a bind."""
    parsed = read_len(message, 1)
    if parsed is None:
        return None
    _, index = parsed
    if index >= len(message) or message[index] != 0x02:
        return None
    parsed = read_len(message, index + 1)
    if parsed is None:
        return None
    length, index = parsed
    index += length
    # APPLICATION 0, constructed. Search and unbind use other tags.
    if index >= len(message) or message[index] != 0x60:
        return None
    parsed = read_len(message, index + 1)
    if parsed is None:
        return None
    _, index = parsed
    if index >= len(message) or message[index] != 0x02:
        return None
    parsed = read_len(message, index + 1)
    if parsed is None:
        return None
    length, index = parsed
    index += length
    if index >= len(message) or message[index] != 0x04:
        return None
    parsed = read_len(message, index + 1)
    if parsed is None:
        return None
    length, index = parsed
    return message[index : index + length]


def bind_message(msgid: int, dn: bytes, password: bytes) -> bytes:
    body = tlv(0x02, bytes([msgid]))
    operation = tlv(
        0x60,
        tlv(0x02, b"\x03") + tlv(0x04, dn) + tlv(0x80, password),
    )
    return tlv(0x30, body + operation)


def search_message(msgid: int, text: bytes) -> bytes:
    body = tlv(0x02, bytes([msgid]))
    operation = tlv(0x63, tlv(0x04, text))
    return tlv(0x30, body + operation)


class Gate:
    """Decide which complete LDAP messages to forward."""

    def __init__(self, reader_dn: bytes) -> None:
        self.reader_dn = reader_dn
        self.binds = 0
        self.buf = b""

    def feed(self, data: bytes) -> list[tuple[str, bytes]]:
        self.buf += data
        actions: list[tuple[str, bytes]] = []
        while self.buf:
            try:
                end = message_end(self.buf)
            except ProxyError:
                self.buf = b""
                actions.append(("fail", b""))
                break
            if end is None:
                if len(self.buf) > 1024 * 1024:
                    self.buf = b""
                    actions.append(("fail", b""))
                break
            message = self.buf[:end]
            self.buf = self.buf[end:]
            try:
                dn = bind_dn(message)
            except ProxyError:
                self.buf = b""
                actions.append(("fail", b""))
                break
            if dn == self.reader_dn:
                self.binds += 1
                if self.binds >= 2:
                    actions.append(("drop", b""))
                    self.buf = b""
                    break
                actions.append(("service-bind", message))
                continue
            actions.append(("send", message))
        return actions


def self_check() -> None:
    reader = b"cn=reader,dc=example,dc=com"
    user = b"uid=ada,ou=people,dc=example,dc=com"
    first = bind_message(1, reader, b"first-secret")
    looked_up = search_message(2, reader)
    user_bind = bind_message(3, user, b"user-secret")
    second = bind_message(4, reader, b"second-secret")
    long_dn = b"cn=" + (b"a" * 180) + b",dc=example,dc=com"
    if bind_dn(bind_message(5, long_dn, b"x" * 200)) != long_dn:
        sys.exit("long bind DN did not round-trip")
    gate = Gate(reader)
    blob = first + looked_up + user_bind + second
    actions: list[tuple[str, bytes]] = []
    for offset in range(0, len(blob), 5):
        actions.extend(gate.feed(blob[offset : offset + 5]))
    sent = b"".join(payload for kind, payload in actions if kind != "drop")
    if gate.binds != 2 or second in sent or b"second-secret" in sent:
        sys.exit("the second service bind was forwarded")
    if first not in sent or user_bind not in sent or looked_up not in sent:
        sys.exit("an earlier LDAP message was dropped")
    if not any(kind == "drop" for kind, _payload in actions):
        sys.exit("the second service bind was not dropped")
    print("ldap proxy self-check ok")


def serve(listen: str, upstream: str, reader_dn: bytes) -> None:
    host, port_text = listen.rsplit(":", 1)
    up_host, up_port_text = upstream.rsplit(":", 1)
    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind((host, int(port_text)))
    server.listen(8)
    print(f"ldap proxy listening on {listen}", file=sys.stderr, flush=True)
    while True:
        client, _addr = server.accept()
        print("accepted a connection", file=sys.stderr, flush=True)
        try:
            handle(client, (up_host, int(up_port_text)), reader_dn)
        except (ProxyError, OSError) as exc:
            print(f"proxy closed the connection: {exc}", file=sys.stderr, flush=True)
        finally:
            client.close()


def handle(client: socket.socket, upstream: tuple[str, int], reader_dn: bytes) -> None:
    remote = socket.create_connection(upstream, timeout=10)
    client.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    remote.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    client.settimeout(None)
    remote.settimeout(None)
    gate = Gate(reader_dn)
    try:
        shuttle(client, remote, gate)
    finally:
        remote.close()


def shuttle(client: socket.socket, remote: socket.socket, gate: Gate) -> None:
    sockets = [client, remote]
    while True:
        readable, _, _ = select.select(sockets, [], [], 30)
        if not readable:
            raise ProxyError("idle timeout")
        if client in readable:
            data = client.recv(65536)
            if not data:
                return
            for action, payload in gate.feed(data):
                if action == "drop":
                    print("dropped the second service bind", file=sys.stderr, flush=True)
                    return
                if action == "fail":
                    print("proxy closed a message it could not parse", file=sys.stderr, flush=True)
                    return
                if action == "service-bind":
                    print("forwarded a service bind", file=sys.stderr, flush=True)
                remote.sendall(payload)
        if remote in readable:
            data = remote.recv(65536)
            if not data:
                return
            client.sendall(data)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-check", action="store_true")
    parser.add_argument("--listen", default="127.0.0.1:1390")
    parser.add_argument("--upstream", default="127.0.0.1:1389")
    parser.add_argument("--reader-dn", default="cn=reader,dc=example,dc=com")
    args = parser.parse_args()
    if args.self_check:
        self_check()
        return
    serve(args.listen, args.upstream, args.reader_dn.encode("utf-8"))


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        sys.exit(130)
    except OSError as exc:
        print(f"ldap proxy failed: {exc}", file=sys.stderr)
        sys.exit(1)

#!/usr/bin/env python3
"""Count and optionally shape bytes on a localhost TCP cache connection.

This forwards the byte stream unchanged. The counters measure application TCP
payload bytes accepted by socket sends: down_bytes is upstream-to-client and
up_bytes is client-to-upstream. TCP/IP headers and retransmissions are outside
the measurement. A nonzero rate applies one aggregate token bucket to all
downstream connections; upstream traffic is never shaped. This proxy adds no
per-RPC latency.
"""

from __future__ import annotations

import math
import select
import socket
import threading
import time
from typing import Optional


_CHUNK_SIZE = 64 * 1024
_POLL_SECONDS = 0.2
_CONNECT_TIMEOUT = 3.0


class _DownlinkLimiter:
    """A process-wide byte token bucket with a small bounded burst."""

    def __init__(self, mib_per_second: float) -> None:
        self.rate = mib_per_second * 1024 * 1024
        # At most 64 KiB of burst, and at most 10 ms of traffic at low rates.
        self.capacity = min(_CHUNK_SIZE, max(1, int(self.rate * 0.01))) if self.rate else 0
        self.tokens = self.capacity
        self.updated = time.monotonic()
        self.condition = threading.Condition()

    def wake(self) -> None:
        with self.condition:
            self.condition.notify_all()

    def _refill(self, now: float) -> None:
        if self.rate:
            self.tokens = min(self.capacity, self.tokens + (now - self.updated) * self.rate)
        self.updated = now

    def acquire(self, requested: int, stop_event: threading.Event,
                connection_stop: threading.Event) -> int:
        if not self.rate:
            return requested
        amount = min(float(requested), self.capacity)
        with self.condition:
            while not stop_event.is_set() and not connection_stop.is_set():
                now = time.monotonic()
                self._refill(now)
                if self.tokens >= amount:
                    self.tokens -= amount
                    return int(amount)
                delay = (amount - self.tokens) / self.rate
                self.condition.wait(min(delay, _POLL_SECONDS))
            return 0

    def refund(self, amount: int) -> None:
        if amount <= 0 or not self.rate:
            return
        with self.condition:
            self._refill(time.monotonic())
            self.tokens = min(self.capacity, self.tokens + amount)
            self.condition.notify_all()


class _Connection:
    def __init__(self, proxy: "WireProxy", client: socket.socket) -> None:
        self.proxy = proxy
        self.client = client
        self.upstream: Optional[socket.socket] = None
        self.lock = threading.Lock()
        self.stopped = threading.Event()

    def attach_upstream(self, upstream: socket.socket) -> bool:
        with self.lock:
            if self.stopped.is_set() or self.proxy._stop_event.is_set():
                keep = False
            else:
                self.upstream = upstream
                keep = True
        if not keep:
            _close_socket(upstream)
        return keep

    def sockets(self) -> tuple[socket.socket, ...]:
        with self.lock:
            if self.upstream is None:
                return (self.client,)
            return (self.client, self.upstream)

    def half_close_write(self, target: socket.socket) -> None:
        try:
            target.shutdown(socket.SHUT_WR)
        except OSError:
            # The other direction may already have closed this socket.
            pass

    def abort(self) -> None:
        self.stopped.set()
        self.proxy._limiter.wake()
        for sock in self.sockets():
            try:
                sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass

    def close(self) -> None:
        for sock in self.sockets():
            _close_socket(sock)


def _close_socket(sock: socket.socket) -> None:
    try:
        sock.close()
    except OSError:
        pass


class WireProxy:
    """Forward a raw TCP stream through a localhost listener.

    Args:
        upstream_host: Hostname or IP address of the cache server.
        upstream_port: TCP port of the cache server.
        mib_per_second: Aggregate downstream limit in MiB/s. Zero disables
            shaping. Uploads are always unshaped.
        port: Local port to bind; zero asks the OS to choose one.

    Call ``start()`` before using ``port``. ``stop()`` is idempotent and joins
    the accept and connection threads before returning.
    """

    def __init__(self, upstream_host: str, upstream_port: int,
                 mib_per_second: float = 100, port: int = 0) -> None:
        if not upstream_host:
            raise ValueError("upstream_host must not be empty")
        if not 1 <= int(upstream_port) <= 65535:
            raise ValueError("upstream_port must be between 1 and 65535")
        if not 0 <= int(port) <= 65535:
            raise ValueError("port must be between 0 and 65535")
        if not math.isfinite(float(mib_per_second)) or mib_per_second < 0:
            raise ValueError("mib_per_second must be a finite nonnegative number")

        self.upstream_host = upstream_host
        self.upstream_port = int(upstream_port)
        self.mib_per_second = float(mib_per_second)
        self.port = int(port)

        self._limiter = _DownlinkLimiter(self.mib_per_second)
        self._stop_event = threading.Event()
        self._stopped_event = threading.Event()
        self._state_lock = threading.Lock()
        self._connections_lock = threading.Lock()
        self._connections: set[_Connection] = set()
        self._workers: set[threading.Thread] = set()
        self._listener: Optional[socket.socket] = None
        self._accept_thread: Optional[threading.Thread] = None
        self._started = False
        self._stop_started = False
        self._stats_lock = threading.Lock()
        self._down_bytes = 0
        self._up_bytes = 0

    def start(self) -> "WireProxy":
        with self._state_lock:
            if self._started or self._stop_event.is_set():
                raise RuntimeError("WireProxy can only be started once")
            listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            try:
                listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                listener.bind(("127.0.0.1", self.port))
                listener.listen()
                listener.settimeout(_POLL_SECONDS)
                self.port = listener.getsockname()[1]
                self._listener = listener
                thread = threading.Thread(target=self._accept_loop,
                                          name="cache-wire-proxy-accept")
                self._accept_thread = thread
                self._started = True
                thread.start()
            except BaseException:
                _close_socket(listener)
                self._listener = None
                raise
        return self

    def snapshot(self) -> dict[str, int]:
        with self._stats_lock:
            return {"down_bytes": self._down_bytes, "up_bytes": self._up_bytes}

    def stop(self) -> None:
        with self._state_lock:
            if self._stop_started:
                wait_for_other_stop = True
                listener = None
                accept_thread = None
            else:
                wait_for_other_stop = False
                self._stop_started = True
                self._stop_event.set()
                listener, self._listener = self._listener, None
                accept_thread = self._accept_thread

        if wait_for_other_stop:
            self._stopped_event.wait()
            return

        try:
            self._limiter.wake()
            if listener is not None:
                _close_socket(listener)

            # The accept loop checks _stop_event while registering each client,
            # so this snapshot contains every connection it successfully owns.
            with self._connections_lock:
                connections = tuple(self._connections)
            for connection in connections:
                connection.abort()

            if accept_thread is not None and accept_thread is not threading.current_thread():
                accept_thread.join()

            while True:
                with self._connections_lock:
                    workers = tuple(self._workers)
                if not workers:
                    break
                for worker in workers:
                    if worker is not threading.current_thread():
                        worker.join()
        finally:
            self._stopped_event.set()

    def _accept_loop(self) -> None:
        listener = self._listener
        if listener is None:
            return
        while not self._stop_event.is_set():
            try:
                client, _ = listener.accept()
            except socket.timeout:
                continue
            except OSError:
                if self._stop_event.is_set():
                    break
                continue

            client.setblocking(False)
            connection = _Connection(self, client)
            with self._connections_lock:
                if self._stop_event.is_set():
                    _close_socket(client)
                    break
                worker = threading.Thread(target=self._serve_connection,
                                          args=(connection,),
                                          name="cache-wire-proxy-connection")
                self._connections.add(connection)
                self._workers.add(worker)
                try:
                    worker.start()
                except BaseException:
                    self._connections.discard(connection)
                    self._workers.discard(worker)
                    connection.close()
                    raise

    def _serve_connection(self, connection: _Connection) -> None:
        children: list[threading.Thread] = []
        try:
            upstream = socket.create_connection(
                (self.upstream_host, self.upstream_port), timeout=_CONNECT_TIMEOUT)
            upstream.setblocking(False)
            if not connection.attach_upstream(upstream):
                return

            client = connection.client
            up_thread = threading.Thread(target=self._pump,
                                         args=(connection, client, upstream, "up"),
                                         name="cache-wire-proxy-up")
            down_thread = threading.Thread(target=self._pump,
                                           args=(connection, upstream, client, "down"),
                                           name="cache-wire-proxy-down")
            for child in (up_thread, down_thread):
                child.start()
                children.append(child)
            for child in children:
                child.join()
        except (OSError, ValueError):
            connection.abort()
        finally:
            if any(child.is_alive() for child in children):
                connection.abort()
                for child in children:
                    child.join()
            connection.close()
            with self._connections_lock:
                self._connections.discard(connection)
                self._workers.discard(threading.current_thread())

    def _pump(self, connection: _Connection, source: socket.socket,
              target: socket.socket, direction: str) -> None:
        pending = b""
        offset = 0
        while not self._stop_event.is_set() and not connection.stopped.is_set():
            try:
                if offset >= len(pending):
                    readable, _, _ = select.select([source], [], [], _POLL_SECONDS)
                    if not readable:
                        continue
                    pending = source.recv(_CHUNK_SIZE)
                    offset = 0
                    if not pending:
                        connection.half_close_write(target)
                        return

                _, writable, _ = select.select([], [target], [], _POLL_SECONDS)
                if not writable:
                    continue

                remaining = len(pending) - offset
                grant = (self._limiter.acquire(remaining, self._stop_event, connection.stopped)
                         if direction == "down" else min(remaining, _CHUNK_SIZE))
                if grant <= 0:
                    return
                try:
                    sent = target.send(memoryview(pending)[offset:offset + grant])
                except (BlockingIOError, InterruptedError):
                    if direction == "down":
                        self._limiter.refund(grant)
                    continue
                except OSError:
                    if direction == "down":
                        self._limiter.refund(grant)
                    if not self._stop_event.is_set() and not connection.stopped.is_set():
                        connection.abort()
                    return

                if sent <= 0:
                    if direction == "down":
                        self._limiter.refund(grant)
                    connection.abort()
                    return
                if direction == "down" and sent < grant:
                    self._limiter.refund(grant - sent)
                self._record(direction, sent)
                offset += sent
            except (OSError, ValueError):
                if not self._stop_event.is_set() and not connection.stopped.is_set():
                    connection.abort()
                return

    def _record(self, direction: str, size: int) -> None:
        with self._stats_lock:
            if direction == "down":
                self._down_bytes += size
            else:
                self._up_bytes += size


__all__ = ["WireProxy"]

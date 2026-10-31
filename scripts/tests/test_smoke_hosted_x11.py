#!/usr/bin/env python3
"""Displayfd protocol fixtures: no X server, subprocess, network or app launch."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location(
    "smoke_hosted_x11", Path(__file__).resolve().parents[1] / "smoke-hosted-x11.py")
SMOKE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SMOKE)


class DisplayReadinessTests(unittest.TestCase):
    def invoke(self, steps, deadline=230, expected=None):
        now = [0.0]
        pending = list(steps)
        reads = []
        waits = []
        registrations = []
        selector = mock.MagicMock()
        selector.__enter__.return_value = selector
        selector.register.side_effect = lambda *args: registrations.append(args)

        def select(timeout):
            waits.append(timeout)
            self.assertGreater(timeout, 0)
            if not pending:
                now[0] += timeout
                return []
            delay, kind, payload = pending.pop(0)
            now[0] += delay
            if kind == "none":
                return []
            if kind == "both":
                return [(SimpleNamespace(data=k), 1) for k in ("display", "exit")]
            if kind == "display":
                reads.append(payload)
            return [(SimpleNamespace(data=kind), 1)]

        def read(fd, count):
            self.assertEqual(fd, 41)
            self.assertLessEqual(count, 7)
            value = reads.pop(0)
            if isinstance(value, tuple):
                value, delay = value
                now[0] += delay
            if isinstance(value, Exception):
                raise value
            self.assertLessEqual(len(value), count)
            return value

        selector.select.side_effect = select
        output = io.StringIO()
        with mock.patch.object(SMOKE.time, "monotonic", side_effect=lambda: now[0]), \
             mock.patch.object(SMOKE.selectors, "DefaultSelector", return_value=selector), \
             mock.patch.object(SMOKE.os, "set_blocking") as nonblocking, \
             mock.patch.object(SMOKE.os, "read", side_effect=read), \
             mock.patch.object(SMOKE.os, "close") as close, \
             mock.patch.object(SMOKE.signal, "pidfd_send_signal") as signal, \
             contextlib.redirect_stdout(output):
            if expected:
                with self.assertRaisesRegex(RuntimeError, expected):
                    SMOKE.wait_display_number(41, 42, deadline)
                result = None
            else:
                result = SMOKE.wait_display_number(41, 42, deadline)
            nonblocking.assert_called_once_with(41, False)
            close.assert_not_called()
            signal.assert_not_called()
        self.assertEqual(registrations, [(41, SMOKE.selectors.EVENT_READ, "display"),
                                         (42, SMOKE.selectors.EVENT_READ, "exit")])
        self.assertTrue(selector.__exit__.called)
        receipt = json.loads(output.getvalue())
        self.assertLessEqual(receipt["response_bytes"], 7)
        return result, receipt, waits

    def test_immediate_valid_number(self):
        result, receipt, _ = self.invoke([(0, "display", b"12\n")])
        self.assertEqual(result, b"12\n")
        self.assertEqual(receipt["xvfb_startup"], "ready")

    def test_fragmented_delayed_response_beyond_old_ten_seconds(self):
        result, receipt, waits = self.invoke([(11, "display", b"1"), (1, "display", b"2\n")])
        self.assertEqual(result, b"12\n")
        self.assertEqual(receipt["elapsed_seconds"], 12)
        self.assertEqual(waits, [30, 19])

    def test_five_digit_boundary(self):
        self.assertEqual(self.invoke([(1, "display", b"12345\n")])[0], b"12345\n")

    def test_eof_rejected(self):
        _, receipt, _ = self.invoke([(1, "display", b"")], expected="EOF")
        self.assertEqual(receipt["xvfb_startup"], "eof")

    def test_partial_then_eof_rejected(self):
        self.invoke([(1, "display", b"1"), (1, "display", b"")], expected="EOF")

    def test_malformed_and_oversized_responses(self):
        for data in (b"\n", b"x\n", b"1\r\n", b"1\n2\n", b"123456", b"123456\n", b"1\x00"):
            with self.subTest(data=data):
                _, receipt, _ = self.invoke([(1, "display", data)], expected="malformed")
                self.assertEqual(receipt["xvfb_startup"], "malformed")

    def test_server_exit_rejected_without_reaping(self):
        _, receipt, _ = self.invoke([(1, "exit", None)], expected="exited")
        self.assertEqual(receipt["xvfb_startup"], "server-exit")

    def test_exit_wins_over_simultaneous_reply(self):
        self.invoke([(1, "both", None)], expected="exited")

    def test_no_reply_hits_startup_cap(self):
        _, receipt, waits = self.invoke([], expected="deadline")
        self.assertEqual(waits, [30])
        self.assertEqual(receipt["elapsed_seconds"], 30)

    def test_original_work_budget_is_not_extended(self):
        _, receipt, waits = self.invoke([], deadline=4, expected="deadline")
        self.assertEqual(waits, [4])
        self.assertEqual(receipt["elapsed_seconds"], 4)

    def test_expired_admission_never_waits(self):
        self.assertEqual(self.invoke([], deadline=0, expected="deadline")[2], [])

    def test_late_reply_is_rejected(self):
        self.invoke([(31, "display", b"1\n")], expected="deadline")

    def test_deadline_after_read_is_rejected(self):
        self.invoke([(29, "display", (b"1\n", 2))], expected="deadline")

    def test_spurious_readiness_and_empty_select_are_bounded(self):
        result, _, waits = self.invoke([(1, "none", None),
            (1, "display", BlockingIOError()), (1, "display", b"0\n")])
        self.assertEqual(result, b"0\n")
        self.assertEqual(waits, [30, 29, 28])

    def test_original_guardian_budgets_preserved(self):
        self.assertEqual((SMOKE.WORK_SECONDS, SMOKE.TOTAL_SECONDS), (230, 240))


if __name__ == "__main__":
    unittest.main()

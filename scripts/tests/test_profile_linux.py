import importlib.util
from pathlib import Path
import unittest


REPO = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "profile_linux", REPO / "scripts/profile-linux.py"
)
PROFILE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROFILE)


def fdinfo(*fields):
    return "\n".join(fields) + "\n"


class LinuxDrmMemoryTests(unittest.TestCase):
    def test_xe_resident_regions_ignore_overlapping_categories(self):
        total, categories, estimated = PROFILE.parse_drm_memory([
            fdinfo(
                "drm-driver:\txe",
                "drm-pdev:\t0000:00:02.0",
                "drm-client-id:\t42",
                "drm-total-system:\t8 MiB",
                "drm-shared-gtt:\t4 MiB",
                "drm-active-gtt:\t1 MiB",
                "drm-resident-system:\t0",
                "drm-total-gtt:\t82028 KiB",
                "drm-resident-gtt:\t82028 KiB",
                "drm-purgeable-gtt:\t12 MiB",
                "drm-resident-stolen:\t0",
            )
        ])

        self.assertEqual(total, 82028 * 1024)
        self.assertEqual(categories, {
            "drm-resident-gtt": 82028 * 1024,
            "drm-resident-stolen": 0,
            "drm-resident-system": 0,
        })
        self.assertFalse(estimated)

    def test_amd_legacy_alias_and_resident_key_are_not_added(self):
        total, categories, estimated = PROFILE.parse_drm_memory([
            fdinfo(
                "drm-pdev: 0000:03:00.0",
                "drm-client-id: 7",
                "drm-memory-vram: 2048 KiB",
                "drm-resident-vram: 2 MiB",
                "drm-memory-gtt: 128 KiB",
            ),
            fdinfo(
                "drm-pdev: 0000:03:00.0",
                "drm-client-id: 7",
                "drm-memory-vram: 2 MiB",
                "drm-resident-gtt: 128 KiB",
            ),
        ])

        self.assertEqual(total, 2 * 1024 * 1024 + 128 * 1024)
        self.assertEqual(categories, {
            "drm-resident-gtt": 128 * 1024,
            "drm-resident-vram": 2 * 1024 * 1024,
        })
        self.assertFalse(estimated)

    def test_modern_resident_key_overrides_legacy_alias_including_zero(self):
        total, categories, estimated = PROFILE.parse_drm_memory([
            fdinfo(
                "drm-pdev: 0000:03:00.0",
                "drm-client-id: 8",
                "drm-memory-vram: 3 MiB",
                "drm-resident-vram: 2 MiB",
                "drm-memory-gtt: 1 MiB",
                "drm-resident-gtt: 0",
            )
        ])

        self.assertEqual(total, 2 * 1024 * 1024)
        self.assertEqual(categories, {
            "drm-resident-gtt": 0,
            "drm-resident-vram": 2 * 1024 * 1024,
        })
        self.assertFalse(estimated)

    def test_independent_clients_sum_but_duplicate_fds_do_not(self):
        total, categories, estimated = PROFILE.parse_drm_memory([
            fdinfo("drm-pdev: 0000:03:00.0", "drm-client-id: 10",
                   "drm-resident-gtt: 100 B", "drm-resident-vram: 2 B"),
            fdinfo("drm-pdev: 0000:03:00.0", "drm-client-id: 10",
                   "drm-resident-gtt: 100 B", "drm-resident-vram: 2 B"),
            fdinfo("drm-pdev: 0000:03:00.0", "drm-client-id: 11",
                   "drm-resident-gtt: 200 B", "drm-resident-vram: 1 B"),
            # Client IDs can be device-scoped, so the same ID on another
            # device is an independent client.
            fdinfo("drm-pdev: 0000:04:00.0", "drm-client-id: 10",
                   "drm-resident-gtt: 50 B"),
        ])

        self.assertEqual(total, 353)
        self.assertEqual(categories, {
            "drm-resident-gtt": 350,
            "drm-resident-vram": 3,
        })
        self.assertFalse(estimated)

    def test_absent_keys_are_unknown_but_reported_zero_is_zero(self):
        self.assertEqual(
            PROFILE.parse_drm_memory([fdinfo("drm-total-gtt: 1 MiB")]),
            (None, {}, None),
        )
        self.assertEqual(
            PROFILE.parse_drm_memory([
                fdinfo("drm-pdev: 0000:03:00.0", "drm-client-id: 12",
                       "drm-resident-gtt: 0", "drm-active-gtt: 9 MiB")
            ]),
            (0, {"drm-resident-gtt": 0}, False),
        )

    def test_units_and_dynamic_region_names(self):
        total, categories, estimated = PROFILE.parse_drm_memory([
            fdinfo(
                "drm-pdev: 0000:03:00.0",
                "drm-client-id: 13",
                "drm-resident-vram0: 7 B",
                "drm-resident-gtt: 2KiB",
                "drm-resident-system: 1 MiB",
                "drm-resident-stolen: 3",
            )
        ])

        self.assertEqual(total, 1_050_634)
        self.assertEqual(categories, {
            "drm-resident-gtt": 2 * 1024,
            "drm-resident-stolen": 3,
            "drm-resident-system": 1024 * 1024,
            "drm-resident-vram0": 7,
        })
        self.assertFalse(estimated)

    def test_missing_client_ids_use_conservative_per_device_maxima(self):
        total, categories, estimated = PROFILE.parse_drm_memory([
            fdinfo("drm-pdev: 0000:03:00.0", "drm-resident-gtt: 100 B"),
            fdinfo("drm-pdev: 0000:03:00.0", "drm-resident-gtt: 200 B",
                   "drm-resident-vram: 10 B"),
            fdinfo("drm-pdev: 0000:04:00.0", "drm-resident-gtt: 300 B"),
        ])

        self.assertEqual(total, 510)
        self.assertEqual(categories, {
            "drm-resident-gtt": 500,
            "drm-resident-vram": 10,
        })
        self.assertTrue(estimated)

        # Without a PCI identity, unknown-device entries also use one
        # conservative bucket rather than assuming the descriptors are unique.
        self.assertEqual(
            PROFILE.parse_drm_memory([
                fdinfo("drm-resident-gtt: 100 B"),
                fdinfo("drm-resident-gtt: 200 B"),
            ]),
            (200, {"drm-resident-gtt": 200}, True),
        )


if __name__ == "__main__":
    unittest.main()

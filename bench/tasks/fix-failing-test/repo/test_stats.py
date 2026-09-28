import unittest

from stats import mean, median


class MeanTest(unittest.TestCase):
    def test_mean(self):
        self.assertEqual(mean([1, 2, 3, 4]), 2.5)

    def test_mean_empty(self):
        with self.assertRaises(ValueError):
            mean([])


class MedianTest(unittest.TestCase):
    def test_odd_sorted(self):
        self.assertEqual(median([1, 2, 3]), 2)

    def test_even(self):
        self.assertEqual(median([1, 2, 3, 4]), 2.5)

    def test_empty(self):
        with self.assertRaises(ValueError):
            median([])


if __name__ == "__main__":
    unittest.main()

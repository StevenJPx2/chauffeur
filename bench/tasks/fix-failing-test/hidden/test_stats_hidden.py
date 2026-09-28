import unittest

from stats import median


class MedianHiddenTest(unittest.TestCase):
    def test_unsorted_odd(self):
        self.assertEqual(median([9, 1, 5]), 5)

    def test_unsorted_even_does_not_mutate(self):
        values = [10, -2, 7, 3]
        self.assertEqual(median(values), 5)
        self.assertEqual(values, [10, -2, 7, 3])


if __name__ == "__main__":
    unittest.main()

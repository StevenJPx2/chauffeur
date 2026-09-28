import unittest

from textutil import slugify


class SlugifyTest(unittest.TestCase):
    def test_basic(self):
        self.assertEqual(slugify("Hello, World!"), "hello-world")

    def test_accents(self):
        self.assertEqual(slugify("  Crème Brûlée  "), "creme-brulee")
        self.assertEqual(slugify("Ñandú Façade Über"), "nandu-facade-uber")

    def test_special_letters(self):
        self.assertEqual(slugify("Straße --- Ærø"), "strasse-aero")
        self.assertEqual(slugify("Œuvre Łódź"), "oeuvre-lodz")

    def test_collapse_and_trim(self):
        self.assertEqual(slugify("--a__b  c--"), "a-b-c")
        self.assertEqual(slugify("***"), "")
        self.assertEqual(slugify(""), "")

    def test_digits_kept(self):
        self.assertEqual(slugify("Version 2.0 Released"), "version-2-0-released")

    def test_other_unicode_dropped(self):
        self.assertEqual(slugify("snow ☃ man"), "snow-man")

    def test_word_boundary_truncation(self):
        text = "the quick brown fox jumps over the lazy dog and keeps running far away"
        result = slugify(text)
        self.assertEqual(result, "the-quick-brown-fox-jumps-over-the-lazy-dog-and-keeps")
        self.assertLessEqual(len(result), 60)

    def test_exact_boundary(self):
        text = ("a" * 29) + " " + ("b" * 30) + " tail"
        self.assertEqual(slugify(text), ("a" * 29) + "-" + ("b" * 30))

    def test_long_single_word(self):
        self.assertEqual(slugify("x" * 80), "x" * 60)


if __name__ == "__main__":
    unittest.main()

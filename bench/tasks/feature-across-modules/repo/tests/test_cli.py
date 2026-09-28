import contextlib
import io

from shop import cli

from .helpers import StoreTestCase


def run(*argv):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = cli.main(list(argv))
    return code, out.getvalue(), err.getvalue()


class CliTest(StoreTestCase):
    def test_checkout(self):
        code, out, _ = run("--store", str(self.path), "checkout", "--customer", "bob", "--item", "MUG:2")
        self.assertEqual(code, 0)
        self.assertIn("total: 25.00", out)

    def test_unknown_product_is_an_error(self):
        code, _, err = run("--store", str(self.path), "checkout", "--customer", "bob", "--item", "NOPE")
        self.assertEqual(code, 1)
        self.assertIn("unknown product", err)

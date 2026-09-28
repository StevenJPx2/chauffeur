import argparse


def main():
    parser = argparse.ArgumentParser(description="Greet someone.")
    parser.add_argument("--name", default="world", help="who to greet")
    parser.add_argument("--verbose", action="store_true", help="print extra output")
    args = parser.parse_args()
    if args.verbose:
        print("verbose on")
    print(f"hello, {args.name}")


if __name__ == "__main__":
    main()

import argparse


def main():
    parser = argparse.ArgumentParser(description="Greet someone.")
    parser.add_argument("--name", default="world", help="who to greet")
    args = parser.parse_args()
    print(f"hello, {args.name}")


if __name__ == "__main__":
    main()

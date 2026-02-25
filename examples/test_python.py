import orionis

def buggy_function():
    a = 10
    b = 0
    return a / b

def intermediate_function():
    x = "hello"
    y = {"user": "test"}
    buggy_function()

def main():
    print("Starting Orionis execution...")
    try:
        intermediate_function()
    except Exception as e:
        print(f"Caught exception: {e}")

if __name__ == "__main__":
    orionis.start(mode="dev")
    main()

extern int theorem_external(int);

int theorem_add(int a, int b) {
    return a + b;
}

int main(void) {
    return theorem_external(theorem_add(40, 2));
}

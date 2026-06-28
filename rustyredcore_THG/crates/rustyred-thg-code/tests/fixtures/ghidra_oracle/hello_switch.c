extern int theorem_external(int value);

int theorem_switch(int selector) {
    switch (selector) {
    case 0:
        return theorem_external(11);
    case 1:
        return theorem_external(13);
    case 2:
        return theorem_external(17);
    case 3:
        return theorem_external(19);
    case 4:
        return theorem_external(23);
    case 5:
        return theorem_external(29);
    case 6:
        return theorem_external(31);
    case 7:
        return theorem_external(37);
    default:
        return theorem_external(41);
    }
}

int main(int argc, char **argv) {
    (void)argv;
    return theorem_switch(argc);
}

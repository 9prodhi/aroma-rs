class Fixture {
    int weight(char c) {
        int w = 0;
        switch (c) {
            case 'a':
            case 'e':
                w = 1;
                break;
            default:
                w = 2;
        }
        return w;
    }
}

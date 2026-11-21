class Fixture {
    void drain(Queue queue) {
        StringBuilder out = new StringBuilder();
        Iterator it = queue.iterator();
        while (it.hasNext()) {
            out.append(it.next());
        }
    }
}

class Fixture {
    Runnable task(final Job job) {
        return new Runnable() {
            @Override
            public void run() {
                job.execute();
            }
        };
    }
}

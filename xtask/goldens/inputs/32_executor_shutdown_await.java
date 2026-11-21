import java.util.List;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;

class ExecutorShutdownAwait {
    boolean runAndWait(List<Runnable> tasks) throws InterruptedException {
        ExecutorService executor = Executors.newFixedThreadPool(4);
        for (Runnable task : tasks) {
            executor.submit(task);
        }
        executor.shutdown();
        boolean done = executor.awaitTermination(30, TimeUnit.SECONDS);
        if (!done) {
            executor.shutdownNow();
        }
        return done;
    }
}

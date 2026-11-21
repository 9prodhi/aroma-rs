import java.util.ArrayList;
import java.util.List;

class ThreadJoinWorkers {
    int runWorkers(List<Runnable> tasks) throws InterruptedException {
        List<Thread> threads = new ArrayList<>();
        for (Runnable task : tasks) {
            Thread thread = new Thread(task);
            thread.start();
            threads.add(thread);
        }
        int joined = 0;
        for (Thread thread : threads) {
            thread.join();
            joined++;
        }
        System.out.println(joined);
        return joined;
    }
}

import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;

class ExecutorSubmitAll {
    int sumAll(List<Integer> inputs) throws Exception {
        ExecutorService executor = Executors.newFixedThreadPool(4);
        List<Future<Integer>> futures = new ArrayList<>();
        for (Integer input : inputs) {
            futures.add(executor.submit(() -> input * input));
        }
        int total = 0;
        for (Future<Integer> future : futures) {
            total += future.get();
        }
        executor.shutdown();
        return total;
    }
}

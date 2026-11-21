import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;

class ExecutorCollectResults {
    List<String> upperAll(List<String> inputs) throws Exception {
        ExecutorService executor = Executors.newFixedThreadPool(2);
        List<Future<String>> futures = new ArrayList<>();
        for (String input : inputs) {
            futures.add(executor.submit(() -> input.toUpperCase()));
        }
        List<String> results = new ArrayList<>();
        for (Future<String> future : futures) {
            results.add(future.get());
        }
        executor.shutdown();
        return results;
    }
}

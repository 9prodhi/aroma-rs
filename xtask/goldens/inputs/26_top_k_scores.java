import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

class TopKScores {
    List<Integer> topK(List<Integer> scores, int k) {
        List<Integer> sorted = new ArrayList<>(scores);
        Collections.sort(sorted, Collections.reverseOrder());
        int size = sorted.size();
        if (size > k) {
            sorted = sorted.subList(0, k);
        }
        System.out.println(size);
        return sorted;
    }
}

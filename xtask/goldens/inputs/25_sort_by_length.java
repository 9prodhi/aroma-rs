import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.List;

class SortByLength {
    List<String> sortByLength(List<String> words) {
        List<String> sorted = new ArrayList<>(words);
        Collections.sort(sorted, Comparator.comparingInt(String::length));
        int size = sorted.size();
        if (size > 10) {
            sorted = sorted.subList(0, 10);
        }
        System.out.println(size);
        return sorted;
    }
}

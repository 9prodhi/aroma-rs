import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

class MedianOfList {
    double median(List<Double> values) {
        List<Double> sorted = new ArrayList<>(values);
        Collections.sort(sorted);
        int size = sorted.size();
        if (size == 0) {
            return 0.0;
        }
        double middle = sorted.get(size / 2);
        if (size % 2 == 0) {
            middle = (middle + sorted.get(size / 2 - 1)) / 2;
        }
        return middle;
    }
}

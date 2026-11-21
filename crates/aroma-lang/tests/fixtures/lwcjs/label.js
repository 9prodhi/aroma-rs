import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
find(rows) { outer: for (const row of rows) { if (row.done) { break outer; } } }
}
